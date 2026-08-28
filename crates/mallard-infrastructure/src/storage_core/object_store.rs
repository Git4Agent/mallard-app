//! The narrow, namespace-agnostic transport contract shared by the repository
//! bundle protocol and the storage-scoped catalog protocol.
//!
//! Implementations are synchronous by design; every call that reaches a
//! remote store must already be on a blocking-capable thread.

use bytes::Bytes;
use mallard_domain::{hex_lower, sha256_hex};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;

use super::{ObjectKey, ObjectPrefix};
use mallard_domain as domain;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMetadata {
    pub bytes: Vec<u8>,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedObject {
    pub size: u64,
    pub sha256: String,
}

struct VerifiedBodyInner {
    file: Mutex<NamedTempFile>,
}

pub struct VerifiedBodyReader {
    inner: Arc<VerifiedBodyInner>,
    position: u64,
}

impl Read for VerifiedBodyReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let mut staged = self
            .inner
            .file
            .lock()
            .map_err(|_| std::io::Error::other("verified object lock is poisoned"))?;
        staged.as_file_mut().seek(SeekFrom::Start(self.position))?;
        let count = staged.as_file_mut().read(buffer)?;
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| std::io::Error::other("verified object reader position overflow"))?;
        Ok(count)
    }
}

/// A fully received and integrity-verified object backed by a private
/// machine-local temporary file. Clones share ownership of the staged bytes;
/// each [`Self::open`] call returns an independent reader at byte zero.
#[derive(Clone)]
pub struct VerifiedBody {
    inner: Arc<VerifiedBodyInner>,
    size: u64,
    sha256: String,
    etag: String,
}

impl std::fmt::Debug for VerifiedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedBody")
            .field("size", &self.size)
            .field("sha256", &self.sha256)
            .field("etag", &self.etag)
            .finish_non_exhaustive()
    }
}

impl VerifiedBody {
    pub(crate) fn from_temp_file(
        mut file: NamedTempFile,
        size: u64,
        sha256: String,
        etag: String,
    ) -> Result<Self, String> {
        domain::validate_sha256("verified object", &sha256)?;
        let actual_size = file
            .as_file()
            .metadata()
            .map_err(|error| format!("inspect verified object: {}", error))?
            .len();
        if actual_size != size {
            return Err("verified object size differs from its staged file".to_string());
        }
        file.as_file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(|error| format!("rewind verified object: {}", error))?;
        Ok(Self {
            inner: Arc::new(VerifiedBodyInner {
                file: Mutex::new(file),
            }),
            size,
            sha256,
            etag,
        })
    }

    pub fn open(&self) -> Result<VerifiedBodyReader, String> {
        Ok(VerifiedBodyReader {
            inner: self.inner.clone(),
            position: 0,
        })
    }

    pub fn read_bounded(&self, max_bytes: u64, label: &str) -> Result<Vec<u8>, String> {
        if self.size > max_bytes {
            return Err(format!("{} exceeds its read limit", label));
        }
        let capacity = usize::try_from(self.size)
            .map_err(|_| format!("{} cannot fit in memory on this platform", label))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| format!("reserve memory for {}", label))?;
        Read::take(self.open()?, max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {}: {}", label, error))?;
        if bytes.len() as u64 != self.size || sha256_hex(&bytes) != self.sha256 {
            return Err(format!("{} changed after verification", label));
        }
        Ok(bytes)
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn etag(&self) -> &str {
        &self.etag
    }
}

#[derive(Clone, Debug)]
pub enum StageOutcome {
    Missing,
    Mismatch,
    Verified(VerifiedBody),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasExpectation {
    Absent,
    Match(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasOutcome {
    Written { etag: String },
    Conflict { current_etag: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImmutablePutOutcome {
    Written,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreListPage {
    pub keys: Vec<ObjectKey>,
    pub next_cursor: Option<String>,
}

/// The narrow transport contract required by both storage protocols.
pub trait ObjectStore: Send + Sync {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String>;

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String>;

    /// Fetch several verified objects; results line up with `requests`.
    /// Remote stores override this with a bounded-concurrency implementation.
    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> Result<Vec<StageOutcome>, String> {
        requests
            .iter()
            .map(|(key, expected)| self.stage_verified(key, expected))
            .collect()
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String>;

    /// Store several immutable objects; outcomes line up with `objects`.
    /// Remote stores override this with a bounded-concurrency implementation.
    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        objects
            .iter()
            .map(|(key, bytes)| self.put_immutable(key, bytes))
            .collect()
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String>;
    /// Delete one exact validated Mallard object. Implementations must make a
    /// missing key an idempotent success; callers never supply raw paths.
    fn delete(&self, key: &ObjectKey) -> Result<(), String>;
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String>;

    /// Used only for local isolation checks; remote stores return `None`.
    fn local_root(&self) -> Option<&Path> {
        None
    }
}

/// Compute a SHA-256 over a streaming reader without loading it whole.
pub(crate) fn sha256_reader(reader: &mut impl Read, label: &str) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {}", label, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::super::LocalObjectStore;
    use super::*;

    #[test]
    fn verified_body_readers_have_independent_positions() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        let key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{:032x}/_objects/{}",
            1,
            sha256_hex(b"abcdef")
        ))
        .unwrap();
        store.put_immutable(&key, b"abcdef").unwrap();
        let outcome = store
            .stage_verified(
                &key,
                &ExpectedObject {
                    size: 6,
                    sha256: sha256_hex(b"abcdef"),
                },
            )
            .unwrap();
        let StageOutcome::Verified(body) = outcome else {
            panic!("object was not staged")
        };

        let mut first = body.open().unwrap();
        let mut second = body.open().unwrap();
        let mut first_prefix = [0_u8; 2];
        let mut second_prefix = [0_u8; 2];
        first.read_exact(&mut first_prefix).unwrap();
        second.read_exact(&mut second_prefix).unwrap();

        assert_eq!(&first_prefix, b"ab");
        assert_eq!(&second_prefix, b"ab");
    }
}
