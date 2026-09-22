//! An in-memory store that offers only put, get and list, with plain
//! overwrite and no conditional write: the least a provider has to be.
//! Tests run the engines over it to prove they need nothing more, and use its
//! counters and faults to observe requests.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;

use super::publication::PublicationCache;
use super::{
    ExpectedObject, ImmutablePutOutcome, ObjectKey, ObjectPrefix, ObjectStore, StageOutcome,
    StoreKeyPage, StoreListPage, StoredMetadata, VerifiedBody, MAX_LIST_PAGE,
    REPOSITORY_COMMITS_PREFIX,
};
use mallard_domain::sha256_hex;

#[derive(Default)]
struct Faults {
    /// Record requests still to fail, and whether the record lands anyway
    /// (a lost acknowledgment).
    record_failures: usize,
    record_lands: bool,
    /// Listings still allowed before every later one fails.
    listings_left: Option<usize>,
    hidden: BTreeSet<ObjectKey>,
    /// Runs once, right before the next record request is served: whatever
    /// another computer does between this one's listing and its write.
    before_record: Option<Box<dyn FnOnce() + Send>>,
}

#[derive(Default)]
struct Shared {
    objects: Mutex<BTreeMap<ObjectKey, Vec<u8>>>,
    faults: Mutex<Faults>,
    listings: AtomicUsize,
    reads: AtomicUsize,
    record_requests: AtomicUsize,
}

/// Clones share one set of objects, like two computers on one storage.
#[derive(Clone, Default)]
pub struct PlainStore {
    shared: Arc<Shared>,
    cache: Option<Arc<PublicationCache>>,
}

impl PlainStore {
    /// Remember listings and records the way the shared store does.
    pub fn with_publication_cache(mut self) -> Self {
        self.cache = Some(Arc::default());
        self
    }

    pub fn reset_counts(&self) {
        self.shared.listings.store(0, Ordering::SeqCst);
        self.shared.reads.store(0, Ordering::SeqCst);
        self.shared.record_requests.store(0, Ordering::SeqCst);
    }

    /// Key listings answered or refused since the last reset.
    pub fn listings(&self) -> usize {
        self.shared.listings.load(Ordering::SeqCst)
    }

    /// Object reads since the last reset.
    pub fn reads(&self) -> usize {
        self.shared.reads.load(Ordering::SeqCst)
    }

    /// Record requests sent since the last reset.
    pub fn record_requests(&self) -> usize {
        self.shared.record_requests.load(Ordering::SeqCst)
    }

    /// Fail the next `count` record requests. With `lands`, the record is
    /// stored and only its acknowledgment is lost.
    pub fn fail_record_requests(&self, count: usize, lands: bool) {
        let mut faults = self.shared.faults.lock().unwrap();
        faults.record_failures = count;
        faults.record_lands = lands;
    }

    /// Answer `count` more listings, then refuse every later one.
    /// `usize::MAX` lifts the fault.
    pub fn fail_listings_after(&self, count: usize) {
        self.shared.faults.lock().unwrap().listings_left = (count != usize::MAX).then_some(count);
    }

    /// Leave `key` out of listings while it still reads: a lagging listing.
    pub fn hide_from_listings(&self, key: &ObjectKey) {
        self.shared
            .faults
            .lock()
            .unwrap()
            .hidden
            .insert(key.clone());
    }

    pub fn reveal_in_listings(&self, key: &ObjectKey) {
        self.shared.faults.lock().unwrap().hidden.remove(key);
    }

    /// Run `action` once, between the next record request's listing and its
    /// write.
    pub fn before_next_record_request(&self, action: impl FnOnce() + Send + 'static) {
        self.shared.faults.lock().unwrap().before_record = Some(Box::new(action));
    }

    pub fn clear(&self) {
        self.shared.objects.lock().unwrap().clear();
        self.shared.faults.lock().unwrap().hidden.clear();
    }

    pub fn keys(&self) -> Vec<ObjectKey> {
        self.shared
            .objects
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    pub fn remove(&self, key: &ObjectKey) {
        self.shared.objects.lock().unwrap().remove(key);
    }

    fn write(&self, key: &ObjectKey, bytes: &[u8]) {
        self.shared
            .objects
            .lock()
            .unwrap()
            .insert(key.clone(), bytes.to_vec());
    }
}

impl ObjectStore for PlainStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.shared.reads.fetch_add(1, Ordering::SeqCst);
        let Some(bytes) = self.shared.objects.lock().unwrap().get(key).cloned() else {
            return Ok(None);
        };
        if bytes.len() as u64 > max_bytes {
            return Err(format!("metadata object '{}' exceeds its read limit", key));
        }
        Ok(Some(StoredMetadata {
            etag: sha256_hex(&bytes),
            bytes,
        }))
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.shared.reads.fetch_add(1, Ordering::SeqCst);
        let Some(bytes) = self.shared.objects.lock().unwrap().get(key).cloned() else {
            return Ok(StageOutcome::Missing);
        };
        let digest = sha256_hex(&bytes);
        if bytes.len() as u64 != expected.size || digest != expected.sha256 {
            return Ok(StageOutcome::Mismatch);
        }
        let mut staged =
            NamedTempFile::new().map_err(|error| format!("create staged object: {error}"))?;
        staged
            .as_file_mut()
            .write_all(&bytes)
            .map_err(|error| format!("stage object '{}': {}", key, error))?;
        Ok(StageOutcome::Verified(VerifiedBody::from_temp_file(
            staged,
            bytes.len() as u64,
            digest.clone(),
            digest,
        )?))
    }

    /// A plain write: whatever is at the key is replaced.
    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        self.write(key, bytes);
        Ok(ImmutablePutOutcome::Written)
    }

    fn put_record(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        self.shared.record_requests.fetch_add(1, Ordering::SeqCst);
        let before = self.shared.faults.lock().unwrap().before_record.take();
        if let Some(before) = before {
            before();
        }
        let lands = {
            let mut faults = self.shared.faults.lock().unwrap();
            if faults.record_failures == 0 {
                None
            } else {
                faults.record_failures -= 1;
                Some(faults.record_lands)
            }
        };
        match lands {
            None => self.put_immutable(key, bytes),
            Some(lands) => {
                if lands {
                    self.write(key, bytes);
                }
                Err("record request failed in the transport".to_string())
            }
        }
    }

    /// Record listings ask for keys only; a listing that wants revisions in
    /// the record namespace is a regression.
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        assert!(
            !prefix.as_str().starts_with(REPOSITORY_COMMITS_PREFIX)
                && !prefix.as_str().ends_with("/_commits"),
            "record listings use list_keys"
        );
        let page = self.list_keys(prefix, cursor, limit)?;
        let objects = self.shared.objects.lock().unwrap();
        Ok(StoreListPage {
            entries: page
                .keys
                .into_iter()
                .map(|key| super::StoreListEntry {
                    etag: objects
                        .get(&key)
                        .map(|bytes| sha256_hex(bytes))
                        .unwrap_or_default(),
                    key,
                })
                .collect(),
            next_cursor: page.next_cursor,
        })
    }

    fn list_keys(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreKeyPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err("list limit is out of range".to_string());
        }
        self.shared.listings.fetch_add(1, Ordering::SeqCst);
        {
            let mut faults = self.shared.faults.lock().unwrap();
            match faults.listings_left.as_mut() {
                Some(0) => return Err("listing failed in the transport".to_string()),
                Some(left) => *left -= 1,
                None => {}
            }
        }
        let under = format!("{}/", prefix.as_str());
        let hidden = self.shared.faults.lock().unwrap().hidden.clone();
        let mut keys = self
            .shared
            .objects
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.as_str().starts_with(&under) && !hidden.contains(*key))
            .filter(|key| cursor.is_none_or(|cursor| key.as_str() > cursor))
            .cloned()
            .collect::<Vec<_>>();
        let next_cursor = (keys.len() > limit).then(|| keys[limit - 1].as_str().to_string());
        keys.truncate(limit);
        Ok(StoreKeyPage { keys, next_cursor })
    }

    fn publication_cache(&self) -> Option<&PublicationCache> {
        self.cache.as_deref()
    }
}
