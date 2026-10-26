//! The narrow, namespace-agnostic transport contract shared by the repository
//! bundle protocol and the storage-scoped catalog protocol.
//!
//! Implementations are synchronous by design; every call that reaches a
//! remote store must already be on a blocking-capable thread.

use bytes::Bytes;
use mallard_domain::{hex_lower, sha256_hex};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use tempfile::{NamedTempFile, TempPath};

use super::local_store::open_read_only_no_follow;
use super::{ObjectKey, ObjectPrefix};
use mallard_domain as domain;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMetadata {
    pub bytes: Vec<u8>,
    pub etag: String,
}

/// Resolve a recursive prefix or one exact metadata key. Folder-based
/// stores can resolve a batch in one traversal of the shared ancestors.
#[derive(Clone, Debug)]
pub enum ObjectQuery {
    Prefix(ObjectPrefix),
    Key(ObjectKey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedObject {
    pub size: u64,
    pub sha256: String,
}

/// The staged file, closed. Only the path is retained between reads: a Pull
/// stages every object of its selection before it writes any, and a process
/// that kept each staged file open would run out of descriptors long before
/// it ran out of staging space. The file is deleted when the last clone of
/// its body drops.
struct VerifiedBodyInner {
    path: TempPath,
    /// The device and inode the bytes were verified under; a reopen that
    /// finds anything else at the path is refused.
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

/// One open read of a staged object. It owns its descriptor and closes it
/// when dropped, so the descriptors in use are exactly the readers alive.
pub struct VerifiedBodyReader {
    file: File,
}

impl Read for VerifiedBodyReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buffer)
    }
}

/// A fully received and integrity-verified object backed by a private
/// machine-local temporary file. Clones share ownership of the staged bytes;
/// each [`Self::open`] call returns an independent reader at byte zero.
/// No descriptor is held between reads.
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
    /// Take ownership of a staged file whose bytes the caller has verified.
    /// The handle is closed here; the path alone is kept.
    pub(crate) fn from_temp_file(
        file: NamedTempFile,
        size: u64,
        sha256: String,
        etag: String,
    ) -> Result<Self, String> {
        domain::validate_sha256("verified object", &sha256)?;
        let metadata = file
            .as_file()
            .metadata()
            .map_err(|error| format!("inspect verified object: {}", error))?;
        if metadata.len() != size {
            return Err("verified object size differs from its staged file".to_string());
        }
        #[cfg(unix)]
        let (device, inode) = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        Ok(Self {
            inner: Arc::new(VerifiedBodyInner {
                path: file.into_temp_path(),
                #[cfg(unix)]
                device,
                #[cfg(unix)]
                inode,
            }),
            size,
            sha256,
            etag,
        })
    }

    /// One body from the verified chunks of a split payload, in order. The
    /// chunks are copied into a new private file while hashing, each read
    /// through `open` so a chunk that changed since its verification is
    /// refused, and the result is refused unless it is exactly `size` bytes
    /// with digest `sha256`.
    pub fn assemble(parts: &[VerifiedBody], size: u64, sha256: &str) -> Result<Self, String> {
        domain::validate_sha256("assembled object", sha256)?;
        let mut file = NamedTempFile::new()
            .map_err(|error| format!("stage assembled object: {}", error))?;
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        for part in parts {
            let mut reader = part.open()?;
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|error| format!("read staged chunk: {}", error))?;
                if count == 0 {
                    break;
                }
                copied = copied
                    .checked_add(count as u64)
                    .ok_or_else(|| "assembled object size overflow".to_string())?;
                if copied > size {
                    return Err("assembled object exceeds its expected size".to_string());
                }
                hasher.update(&buffer[..count]);
                file.as_file_mut()
                    .write_all(&buffer[..count])
                    .map_err(|error| format!("write assembled object: {}", error))?;
            }
        }
        if copied != size || hex_lower(&hasher.finalize()) != sha256 {
            return Err("assembled object failed size/hash verification".to_string());
        }
        file.as_file_mut()
            .flush()
            .map_err(|error| format!("flush assembled object: {}", error))?;
        Self::from_temp_file(file, size, sha256.to_string(), sha256.to_string())
    }

    /// Reopen the staged file for one read. The reader is refused unless the
    /// path still holds the regular file the bytes were verified in, at its
    /// verified size.
    pub fn open(&self) -> Result<VerifiedBodyReader, String> {
        let file = open_read_only_no_follow(&self.inner.path, "verified object")?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect verified object: {}", error))?;
        if !metadata.is_file() || metadata.len() != self.size {
            return Err("verified object changed after verification".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != self.inner.device || metadata.ino() != self.inner.inode {
                return Err("verified object was replaced after verification".to_string());
            }
        }
        Ok(VerifiedBodyReader { file })
    }

    /// Where the staged bytes live, for tests of their lifetime.
    #[cfg(test)]
    fn staged_path(&self) -> &Path {
        &self.inner.path
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
pub enum ImmutablePutOutcome {
    Written,
    AlreadyPresent,
}

/// The answer to one claim request. A claim is the one key two computers
/// reach with different bytes, and the store admits exactly one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The key was free, or already held these bytes: the claim is this
    /// writer's.
    Mine,
    /// The key holds other bytes: another writer's claim, returned as found.
    Taken(Vec<u8>),
}

/// An operation-local read address. It is never serialized or cached.
#[derive(Clone, PartialEq, Eq)]
pub struct ListedObject {
    pub(crate) key: ObjectKey,
    pub(crate) feishu: Option<FeishuReadAddress>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FeishuReadAddress {
    pub root: String,
    pub api: String,
    pub token: String,
}

impl ListedObject {
    pub fn by_key(key: ObjectKey) -> Self {
        Self { key, feishu: None }
    }

    pub fn key(&self) -> &ObjectKey {
        &self.key
    }
}

impl std::fmt::Debug for ListedObject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ListedObject")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// One listed object with the opaque revision the store reported for it.
/// A later conditional delete binds to exactly this revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreListEntry {
    pub key: ObjectKey,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreListPage {
    pub entries: Vec<StoreListEntry>,
    pub next_cursor: Option<String>,
}

impl StoreListPage {
    pub fn keys(&self) -> impl Iterator<Item = &ObjectKey> {
        self.entries.iter().map(|entry| &entry.key)
    }
}

/// One page of a listing that carries keys only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreKeyPage {
    pub keys: Vec<ObjectKey>,
    pub next_cursor: Option<String>,
}

/// The outcome of one conditional delete. There is no unconditional
/// variant: every deletion names the revision it listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    Missing,
    Conflict { current_etag: String },
}

/// The narrow transport contract required by both storage protocols. A
/// provider needs three operations for ordinary sync: write bytes at a key,
/// read a key, list a prefix. There is no conditional write.
pub trait ObjectStore: Send + Sync {
    fn managed_cloud(&self) -> Option<&crate::managed_cloud::ManagedObjectStore> {
        None
    }

    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String>;

    /// Fetch several small objects' bytes; results line up with `keys`.
    /// Remote stores override this with a bounded-concurrency implementation.
    fn get_metadata_many(
        &self,
        keys: &[ObjectKey],
        max_bytes: u64,
    ) -> Result<Vec<Option<StoredMetadata>>, String> {
        keys.iter()
            .map(|key| self.get_metadata(key, max_bytes))
            .collect()
    }

    /// Read listed addresses in order. Key-addressed stores retain their bulk read.
    fn get_listed_metadata_many(
        &self,
        objects: &[ListedObject],
        max_bytes: u64,
    ) -> Result<Vec<Option<StoredMetadata>>, String> {
        if objects.iter().any(|object| object.feishu.is_some()) {
            return Err("listed object belongs to another storage provider".to_string());
        }
        let keys = objects
            .iter()
            .map(|object| object.key.clone())
            .collect::<Vec<_>>();
        self.get_metadata_many(&keys, max_bytes)
    }

    /// Results follow query order, with an independent error per query.
    /// An exact key may be absent when read. No result is retained between
    /// calls. The object limit applies separately to each prefix.
    fn list_objects_batch(
        &self,
        queries: &[ObjectQuery],
        max_objects: usize,
    ) -> Vec<Result<Vec<ListedObject>, String>> {
        queries
            .iter()
            .map(|query| match query {
                ObjectQuery::Prefix(prefix) => self.list_objects(prefix, max_objects),
                ObjectQuery::Key(key) => Ok(vec![ListedObject::by_key(key.clone())]),
            })
            .collect()
    }

    /// A complete bounded scan. Stores that already walk the whole prefix
    /// override this to return that walk once, without synthetic result pages.
    fn list_objects(
        &self,
        prefix: &ObjectPrefix,
        max_objects: usize,
    ) -> Result<Vec<ListedObject>, String> {
        let mut objects = Vec::new();
        let mut cursor = None;
        let mut cursors = std::collections::HashSet::new();
        loop {
            let page = self.list_keys(prefix, cursor.as_deref(), super::MAX_LIST_PAGE)?;
            if objects.len().saturating_add(page.keys.len()) > max_objects {
                return Err(
                    "publication listing exceeds its object limit; the view is incomplete"
                        .to_string(),
                );
            }
            objects.extend(page.keys.into_iter().map(ListedObject::by_key));
            match page.next_cursor {
                Some(next) if cursors.insert(next.clone()) => cursor = Some(next),
                Some(_) => return Err("publication listing repeated a cursor".to_string()),
                None => return Ok(objects),
            }
        }
    }

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

    /// Fetch every verified object of one operation, reporting each
    /// request's expected size once it is settled, whatever its outcome, in
    /// the order the store settles them. Outcomes line up with `requests`.
    /// The default sends one `stage_many_verified` per window of the
    /// transfer-concurrency setting, as the upload path does; a store that
    /// pays per folder rather than per object overrides this to resolve
    /// every request together.
    fn stage_many_with_progress(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
        on_staged: &mut dyn FnMut(u64),
    ) -> Result<Vec<StageOutcome>, String> {
        let window = crate::app_settings::cloud_transfer_concurrency().max(1);
        let mut outcomes = Vec::with_capacity(requests.len());
        for batch in requests.chunks(window) {
            outcomes.extend(self.stage_many_verified(batch)?);
            for (_, expected) in batch {
                on_staged(expected.size);
            }
        }
        Ok(outcomes)
    }

    /// Store one object. No conforming writer ever writes different bytes to
    /// the same key: payloads are content-addressed, a manifest's key holds a
    /// commit id minted per publication, a record's name holds the digest of
    /// its bytes, and the marker and the Library tag are constant. A plain
    /// write is therefore enough; a store may keep a native create-only write.
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

    /// Store every pending object of one operation, reporting each object's
    /// size once it is settled, present or written, in the order the store
    /// settles them. The default sends one `put_many` per window of the
    /// transfer-concurrency setting: the window and a store's bulk semaphore
    /// read the same value, so effective parallelism is their minimum and
    /// they only move together. A store that pays per folder rather than per
    /// object overrides this to take the whole list at once.
    fn put_many_with_progress(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
        on_stored: &mut dyn FnMut(u64),
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        let window = crate::app_settings::cloud_transfer_concurrency().max(1);
        let mut outcomes = Vec::with_capacity(objects.len());
        for batch in objects.chunks(window) {
            outcomes.extend(self.put_many(batch.to_vec())?);
            for (_, bytes) in batch {
                on_stored(bytes.len() as u64);
            }
        }
        Ok(outcomes)
    }

    /// Write one publication record: one request, with no retry inside the
    /// transport. Every attempt to write a record follows a listing of its
    /// stream, and a transport retry would be an attempt without one. A
    /// record's name contains the digest of its bytes, so a key that is
    /// already taken holds these bytes.
    fn put_record(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        self.put_immutable(key, bytes)
    }

    /// Whether this store's server refuses a write to a key that already
    /// holds other bytes, in the same request. Only such a store can
    /// arbitrate a claim between two computers: a folder a sync product
    /// shares would take two claims, and a network share promises no
    /// create-only write. `false` unless the behavior is proved; the local
    /// store never claims.
    fn supports_claims(&self) -> bool {
        false
    }

    /// One create-only request at the claim key, with no retry inside the
    /// transport: `Mine` when the key was free or already held these bytes,
    /// `Taken` with the bytes found otherwise. Used only for the claim a
    /// publication makes of its parent; callers ask `supports_claims` first.
    fn put_claim(&self, _key: &ObjectKey, _bytes: &[u8]) -> Result<ClaimOutcome, String> {
        Err("this storage cannot arbitrate a claim".to_string())
    }

    /// Whether this store can atomically delete only the object revision that
    /// the caller names. Stores return false unless this behavior is proved.
    fn supports_conditional_delete(&self) -> bool {
        false
    }

    /// Delete one exact validated Mallard object only if its current
    /// revision equals `expected_etag`, the revision a listing reported.
    /// A missing key reports `Missing`; a different revision reports
    /// `Conflict` and deletes nothing. Callers never supply raw paths.
    fn delete(&self, _key: &ObjectKey, _expected_etag: &str) -> Result<DeleteOutcome, String> {
        Err("conditional object deletion is not supported by this storage".to_string())
    }

    /// Explicit storage-wide clear. Normal object deletion remains revision checked.
    fn clear_managed_data(&self) -> Result<super::ManagedStorageClearResult, String> {
        if !self.supports_conditional_delete() {
            return Err("storage clear is unavailable for this provider".to_string());
        }
        super::clear_managed_data_with(self, |entry| self.delete(&entry.key, &entry.etag))
    }

    /// Bounded, deterministic listing. Every entry carries the object's
    /// current opaque revision so a caller can delete exactly what it saw;
    /// only storage clear needs that. Publication scans retain read addresses. A name
    /// the store cannot represent as a key is skipped and reported, never a
    /// failed listing. A completed write is visible to every later listing of
    /// its prefix.
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String>;

    /// `list` without revisions. The default complete scan uses this because
    /// a record's name already carries its digest. A store whose `list` pays
    /// per entry for a revision overrides this.
    fn list_keys(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreKeyPage, String> {
        let page = self.list(prefix, cursor, limit)?;
        Ok(StoreKeyPage {
            keys: page.entries.into_iter().map(|entry| entry.key).collect(),
            next_cursor: page.next_cursor,
        })
    }

    /// What this process has listed and read of the store's publication
    /// streams. The shared store keeps one for its lifetime; a bare store
    /// keeps none and reads every time.
    fn publication_cache(&self) -> Option<&super::publication::PublicationCache> {
        None
    }

    /// Used only for local isolation checks; remote stores return `None`.
    fn local_root(&self) -> Option<&Path> {
        None
    }

    /// What the transport last observed about its credential. Stores that
    /// track nothing return `None`; the shared store refuses requests while
    /// the recorded health suspends the connection.
    fn connection_health(&self) -> Option<super::ConnectionHealth> {
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
    use std::sync::Mutex;

    fn repository_object_key(bytes: &[u8]) -> ObjectKey {
        ObjectKey::parse(format!(
            ".mallard/v1/repositories/{:032x}/_objects/{}",
            1,
            sha256_hex(bytes)
        ))
        .unwrap()
    }

    fn stage(store: &LocalObjectStore, bytes: &[u8]) -> VerifiedBody {
        let outcome = store
            .stage_verified(
                &repository_object_key(bytes),
                &ExpectedObject {
                    size: bytes.len() as u64,
                    sha256: sha256_hex(bytes),
                },
            )
            .unwrap();
        let StageOutcome::Verified(body) = outcome else {
            panic!("object was not staged")
        };
        body
    }

    /// The descriptors this process holds open right now.
    #[cfg(unix)]
    fn open_descriptor_count() -> usize {
        std::fs::read_dir("/dev/fd").unwrap().count()
    }

    /// A Pull stages its whole selection before it writes anything, so the
    /// bodies it holds must cost no descriptors: the launcher's soft limit
    /// on macOS is 256, and a selection of small files reaches it long
    /// before the staging-space bound does. The selection here is larger
    /// than that limit.
    #[cfg(unix)]
    #[test]
    fn held_bodies_cost_no_descriptors() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        let payloads: Vec<Vec<u8>> = (0..300)
            .map(|index| format!("object {index}").into_bytes())
            .collect();
        for bytes in &payloads {
            store
                .put_immutable(&repository_object_key(bytes), bytes)
                .unwrap();
        }

        let before = open_descriptor_count();
        let bodies: Vec<VerifiedBody> = payloads.iter().map(|bytes| stage(&store, bytes)).collect();
        let held = open_descriptor_count().saturating_sub(before);
        // Other tests in this process may open a few files meanwhile; the
        // old behavior held one descriptor per body.
        assert!(
            held < bodies.len() / 4,
            "{held} descriptors held for {} staged bodies",
            bodies.len()
        );

        // A reader costs one descriptor while it is alive and reads the
        // verified bytes; the body outlives it.
        let mut reader = bodies[7].open().unwrap();
        let mut read = Vec::new();
        reader.read_to_end(&mut read).unwrap();
        drop(reader);
        assert_eq!(read, payloads[7]);
        assert_eq!(bodies[7].read_bounded(64, "object").unwrap(), payloads[7]);
    }

    #[test]
    fn the_staged_file_lives_with_the_last_clone_of_its_body() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        store
            .put_immutable(&repository_object_key(b"abcdef"), b"abcdef")
            .unwrap();
        let body = stage(&store, b"abcdef");
        let path = body.staged_path().to_path_buf();
        let clone = body.clone();
        drop(body);
        assert!(path.is_file(), "a live clone keeps the staged file");

        // A reader taken before the last clone drops still reads the bytes.
        let mut reader = clone.open().unwrap();
        drop(clone);
        let mut read = Vec::new();
        reader.read_to_end(&mut read).unwrap();
        assert_eq!(read, b"abcdef");
        assert!(!path.exists(), "the last clone deletes the staged file");
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_staged_file_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        store
            .put_immutable(&repository_object_key(b"abcdef"), b"abcdef")
            .unwrap();
        let body = stage(&store, b"abcdef");
        let path = body.staged_path().to_path_buf();
        // Same size, other bytes, another inode: the body's verification no
        // longer covers what the path holds.
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"uvwxyz").unwrap();

        let error = body.open().err().expect("reopen is refused");
        assert!(error.contains("replaced"), "{error}");
    }

    /// A split payload comes back as one body: the chunks are copied in
    /// order while hashing, and the result is refused unless it is exactly
    /// the whole file. A chunk replaced after its verification is refused
    /// through the same reopen check every body makes.
    #[test]
    fn chunks_assemble_into_one_verified_body_or_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        let whole = (0..200_000u32)
            .map(|index| (index.wrapping_mul(31) ^ (index >> 7)) as u8)
            .collect::<Vec<u8>>();
        let (first, second) = whole.split_at(131_072);
        for chunk in [first, second] {
            store
                .put_immutable(&repository_object_key(chunk), chunk)
                .unwrap();
        }
        let parts = [stage(&store, first), stage(&store, second)];
        let body = VerifiedBody::assemble(&parts, whole.len() as u64, &sha256_hex(&whole)).unwrap();
        assert_eq!(body.size(), whole.len() as u64);
        assert_eq!(body.sha256(), sha256_hex(&whole));
        let mut assembled = Vec::new();
        body.open().unwrap().read_to_end(&mut assembled).unwrap();
        assert_eq!(assembled, whole);
        drop(parts);
        // The assembled file outlives the chunks it was built from.
        assert_eq!(body.read_bounded(whole.len() as u64, "assembled").unwrap(), whole);

        let parts = [stage(&store, first), stage(&store, second)];
        let short = VerifiedBody::assemble(&parts, whole.len() as u64 - 1, &sha256_hex(&whole)).unwrap_err();
        assert!(short.contains("exceeds its expected size"), "{short}");
        let long = VerifiedBody::assemble(&parts, whole.len() as u64 + 1, &sha256_hex(&whole)).unwrap_err();
        assert!(long.contains("failed size/hash verification"), "{long}");
        let reordered = [stage(&store, second), stage(&store, first)];
        let wrong_order =
            VerifiedBody::assemble(&reordered, whole.len() as u64, &sha256_hex(&whole)).unwrap_err();
        assert!(wrong_order.contains("failed size/hash verification"), "{wrong_order}");
        let path = parts[1].staged_path().to_path_buf();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, second).unwrap();
        let replaced = VerifiedBody::assemble(&parts, whole.len() as u64, &sha256_hex(&whole)).unwrap_err();
        assert!(replaced.contains("replaced"), "{replaced}");
    }

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

    /// Records the size of every `put_many` and `stage_many_verified`
    /// window it receives.
    struct WindowedStore {
        inner: LocalObjectStore,
        windows: Mutex<Vec<usize>>,
        stage_windows: Mutex<Vec<usize>>,
    }

    impl ObjectStore for WindowedStore {
        fn get_metadata(
            &self,
            key: &ObjectKey,
            max_bytes: u64,
        ) -> Result<Option<StoredMetadata>, String> {
            self.inner.get_metadata(key, max_bytes)
        }

        fn stage_verified(
            &self,
            key: &ObjectKey,
            expected: &ExpectedObject,
        ) -> Result<StageOutcome, String> {
            self.inner.stage_verified(key, expected)
        }

        fn stage_many_verified(
            &self,
            requests: &[(ObjectKey, ExpectedObject)],
        ) -> Result<Vec<StageOutcome>, String> {
            self.stage_windows.lock().unwrap().push(requests.len());
            self.inner.stage_many_verified(requests)
        }

        fn put_immutable(
            &self,
            key: &ObjectKey,
            bytes: &[u8],
        ) -> Result<ImmutablePutOutcome, String> {
            self.inner.put_immutable(key, bytes)
        }

        fn put_many(
            &self,
            objects: Vec<(ObjectKey, Bytes)>,
        ) -> Result<Vec<ImmutablePutOutcome>, String> {
            self.windows.lock().unwrap().push(objects.len());
            self.inner.put_many(objects)
        }

        fn list(
            &self,
            prefix: &ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<StoreListPage, String> {
            self.inner.list(prefix, cursor, limit)
        }
    }

    #[test]
    fn the_default_progress_upload_sends_transfer_windows_and_reports_each_object() {
        let temp = tempfile::tempdir().unwrap();
        let store = WindowedStore {
            inner: LocalObjectStore::open(temp.path().join("store")).unwrap(),
            windows: Mutex::new(Vec::new()),
            stage_windows: Mutex::new(Vec::new()),
        };
        let window = crate::app_settings::cloud_transfer_concurrency();
        let objects: Vec<(ObjectKey, Bytes)> = (0..window * 2 + 1)
            .map(|index| {
                let bytes = Bytes::from(format!("object {index}"));
                let key = ObjectKey::parse(format!(
                    ".mallard/v1/repositories/{:032x}/_objects/{}",
                    1,
                    sha256_hex(&bytes)
                ))
                .unwrap();
                (key, bytes)
            })
            .collect();
        let sizes: Vec<u64> = objects
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .collect();

        let mut reported = Vec::new();
        let outcomes = store
            .put_many_with_progress(objects.clone(), &mut |size| reported.push(size))
            .unwrap();
        assert_eq!(outcomes, vec![ImmutablePutOutcome::Written; objects.len()]);
        assert_eq!(*store.windows.lock().unwrap(), vec![window, window, 1]);
        assert_eq!(reported, sizes, "each object is reported once, in order");

        // A present object is reported like any other and sent nowhere twice.
        reported.clear();
        let outcomes = store
            .put_many_with_progress(objects[..1].to_vec(), &mut |size| reported.push(size))
            .unwrap();
        assert_eq!(outcomes, vec![ImmutablePutOutcome::AlreadyPresent]);
        assert_eq!(reported, sizes[..1]);
    }

    #[test]
    fn the_default_progress_staging_sends_transfer_windows_and_keeps_request_order() {
        let temp = tempfile::tempdir().unwrap();
        let store = WindowedStore {
            inner: LocalObjectStore::open(temp.path().join("store")).unwrap(),
            windows: Mutex::new(Vec::new()),
            stage_windows: Mutex::new(Vec::new()),
        };
        let window = crate::app_settings::cloud_transfer_concurrency();
        let objects: Vec<(ObjectKey, Bytes)> = (0..window * 2)
            .map(|index| {
                let bytes = Bytes::from(format!("object {index}"));
                let key = ObjectKey::parse(format!(
                    ".mallard/v1/repositories/{:032x}/_objects/{}",
                    1,
                    sha256_hex(&bytes)
                ))
                .unwrap();
                (key, bytes)
            })
            .collect();
        store.put_many(objects.clone()).unwrap();
        let mut requests: Vec<(ObjectKey, ExpectedObject)> = objects
            .iter()
            .map(|(key, bytes)| {
                (
                    key.clone(),
                    ExpectedObject {
                        size: bytes.len() as u64,
                        sha256: sha256_hex(bytes),
                    },
                )
            })
            .collect();
        // An absent object in the middle settles as missing in its place.
        let absent = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{:032x}/_objects/{}",
            1,
            "0".repeat(64)
        ))
        .unwrap();
        requests.insert(
            1,
            (
                absent,
                ExpectedObject {
                    size: 3,
                    sha256: "0".repeat(64),
                },
            ),
        );
        let sizes: Vec<u64> = requests.iter().map(|(_, expected)| expected.size).collect();

        let mut reported = Vec::new();
        let outcomes = store
            .stage_many_with_progress(&requests, &mut |size| reported.push(size))
            .unwrap();
        assert_eq!(
            *store.stage_windows.lock().unwrap(),
            vec![window, window, 1]
        );
        assert_eq!(reported, sizes, "each request is reported once, in order");
        assert_eq!(outcomes.len(), requests.len());
        for (index, ((_, expected), outcome)) in requests.iter().zip(&outcomes).enumerate() {
            if index == 1 {
                assert!(matches!(outcome, StageOutcome::Missing));
                continue;
            }
            let StageOutcome::Verified(body) = outcome else {
                panic!("object {index} was not staged")
            };
            assert_eq!(body.sha256(), expected.sha256);
        }
    }
}
