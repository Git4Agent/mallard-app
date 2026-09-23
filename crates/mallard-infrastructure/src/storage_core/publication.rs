//! Publication over immutable records, shared by the bundle engine and the
//! Library engine: list a stream, select its current publication, read a
//! record, and write one.
//!
//! There is no mutable head. A stream's current publication is selected from
//! the names of one listing, and a record is written only right after a
//! listing that shows its parent selected and childless (list, then write).
//! On a store whose server refuses a second write of a key, the parent is
//! claimed first: one create-only write of the record's bytes under the
//! parent's token, which admits one child per parent (claim, then write).
//! A write is a publication only when the listing after it selects the
//! record; otherwise the push failed and the record is never selected. The
//! claim and the record are sent once each: a request the store does not
//! acknowledge is settled by one read-only check, never by sending again,
//! and recovery is the user's. Nothing here is persisted: the process
//! remembers the last listing of a stream and the records it has read, and
//! nothing else.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use mallard_domain::{sha256_hex, BundleId, Mainline, PublicationToken, RecordName};
use serde::Deserialize;
use tokio::sync::broadcast;

use super::{
    library_object_key, ClaimOutcome, ObjectKey, ObjectPrefix, ObjectStore, MAX_LIST_PAGE,
    REPOSITORY_CLAIMS_PREFIX, REPOSITORY_COMMITS_PREFIX,
};

/// A record is at most this large.
pub const MAX_RECORD_BYTES: u64 = 64 * 1024;
/// The listing budget of one stream, and of bundle discovery. Reaching it
/// reports an incomplete view; history is never truncated silently.
const MAX_LISTED_RECORDS: usize = 100_000;
/// A record is sent only while the listing that cleared it is at most this
/// old on the wall clock. The monotonic clock is not enough: on macOS it
/// stands still while the machine sleeps.
const LISTING_FRESHNESS: Duration = Duration::from_secs(10);
/// Listings one operation may make before it gives up on a fresh one. The
/// claim and the record are sent once each, whatever the listings cost.
const MAX_WRITE_LISTINGS: usize = 8;
const MAX_CACHED_RECORDS_PER_STREAM: usize = 64;
const MAX_REPORTED_NAMES: usize = 1_024;

/// Another computer published on the same parent while this push ran, and
/// the push failed as a whole. Either the listing before the write showed
/// the change and nothing was written, or the record was written and another
/// computer's ranks first: that record is kept in the storage and is never
/// selected. Nothing of this push is current either way.
pub const REMOTE_CHANGED: &str = "The storage changed while pushing. Compare again.";
/// The storage could not be reached after the record was sent.
pub const OUTCOME_UNKNOWN: &str = "Couldn't confirm the push. Refresh to see whether it arrived.";
/// A push onto the selected version claimed it and never wrote its record:
/// another computer's, or this computer's for other content. Nothing writes
/// that record by itself; finishing it is the user's. The services print
/// the Push review's sentence with the storage's name
/// ([`unfinished_push_sentence`]).
pub const PUSH_UNFINISHED: &str = "A push onto this version did not finish.";
pub const OWN_PUSH_UNFINISHED: &str =
    "This computer's earlier push onto this version did not finish.";

/// The Push review's sentence for an unfinished push, with the storage's
/// name (section 13 of the CAS-free plan). Any other error passes through.
pub fn unfinished_push_sentence(storage_name: &str, error: String) -> String {
    if error == PUSH_UNFINISHED {
        format!("A push to {storage_name} did not finish. Finish it, then compare again.")
    } else if error == OWN_PUSH_UNFINISHED {
        format!(
            "This computer's earlier push to {storage_name} did not finish. Finish it, then compare again."
        )
    } else {
        error
    }
}

/// Object names the storage holds inside a Mallard namespace that are not
/// Mallard's: a `.DS_Store`, a sync client's conflicted copy. They are
/// ignored, never a record, and announced here once per process so the
/// desktop can write them to the activity log.
pub fn ignored_name_events() -> &'static broadcast::Sender<String> {
    static EVENTS: OnceLock<broadcast::Sender<String>> = OnceLock::new();
    EVENTS.get_or_init(|| broadcast::channel(64).0)
}

pub(crate) fn report_ignored_name(name: &str) {
    static REPORTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let name = name.chars().take(512).collect::<String>();
    let Ok(mut reported) = REPORTED.get_or_init(Default::default).lock() else {
        return;
    };
    if reported.len() >= MAX_REPORTED_NAMES || !reported.insert(name.clone()) {
        return;
    }
    let _ = ignored_name_events().send(name);
}

/// One stream's record namespace: one bundle, or one Library repository.
/// Streams never compete with one another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordStream {
    prefix: ObjectPrefix,
    /// The stream's claims, beside its records and never among them: no
    /// record listing returns one.
    claims: ObjectPrefix,
}

impl RecordStream {
    /// Project records live in the flat namespace beside the bundle folders.
    pub fn project(bundle_id: &BundleId) -> Result<Self, String> {
        Ok(Self {
            prefix: ObjectPrefix::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{bundle_id}"))?,
            claims: ObjectPrefix::parse(format!("{REPOSITORY_CLAIMS_PREFIX}/{bundle_id}"))?,
        })
    }

    /// Library records stay inside their repository, which is never
    /// enumerated: its id is derived from the registered coordinate.
    pub fn library(repository_id: &str) -> Result<Self, String> {
        let directory = |relative: &str| -> Result<ObjectPrefix, String> {
            let probe = library_object_key(repository_id, &format!("{relative}/probe"))?;
            let prefix = probe
                .as_str()
                .strip_suffix("/probe")
                .expect("the Library key constructor appends the relative key");
            ObjectPrefix::parse(prefix)
        };
        Ok(Self {
            prefix: directory("_commits")?,
            claims: directory("_claims")?,
        })
    }

    pub fn key(&self, name: &RecordName) -> Result<ObjectKey, String> {
        ObjectKey::parse(format!("{}/{}", self.prefix.as_str(), name.file_name()))
    }

    /// The claim of `parent` in this stream, or of the virtual root: it
    /// holds the record of that parent's one child.
    pub fn claim_key(&self, parent: Option<&PublicationToken>) -> Result<ObjectKey, String> {
        ObjectKey::parse(format!(
            "{}/{}.json",
            self.claims.as_str(),
            parent_label(parent)
        ))
    }
}

fn parent_label(parent: Option<&PublicationToken>) -> String {
    parent.map_or_else(|| "root".to_string(), PublicationToken::to_string)
}

/// The record a taken claim holds: what an earlier push onto the same parent
/// built, and did not write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedRecord {
    pub name: RecordName,
    pub bytes: Vec<u8>,
}

/// What one write of a record ended in.
#[derive(Clone, Debug)]
pub enum WriteOutcome {
    /// Written and selected: the listing after the write selects the record.
    Stored(StreamView),
    /// Nothing was written. The claim of the parent holds a record that no
    /// listing holds, so a push onto this version did not finish. The
    /// caller decides whether it is this computer's own, for the same
    /// content, and then writes it through [`write_claimed_record`];
    /// anything else is the user's to finish.
    Unfinished(ClaimedRecord),
}

impl WriteOutcome {
    /// The listing that selects the record, or the unfinished push as an
    /// error.
    pub fn stored(self) -> Result<StreamView, String> {
        match self {
            Self::Stored(view) => Ok(view),
            Self::Unfinished(_) => Err(PUSH_UNFINISHED.to_string()),
        }
    }
}

/// The five graph facts a record's name repeats. Both record types carry
/// them under these names, so a claim's record is named here without either
/// engine's record type.
#[derive(Deserialize)]
struct GraphFields {
    revision: u64,
    commit_id: String,
    #[serde(default)]
    previous_commit_id: Option<String>,
    #[serde(default)]
    previous_record_sha256: Option<String>,
    created_at_ms: u64,
}

/// The name of the record `bytes` are, when they are a record of `parent`:
/// the five graph facts they state, and the digest of the bytes themselves.
/// Anything else is not a record of that parent.
pub fn record_name_of(
    bytes: &[u8],
    parent: Option<&PublicationToken>,
) -> Result<RecordName, String> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("the record exceeds its size limit".to_string());
    }
    let fields: GraphFields = serde_json::from_slice(bytes)
        .map_err(|error| format!("not a publication record: {error}"))?;
    let named_parent = match (fields.previous_commit_id, fields.previous_record_sha256) {
        (None, None) => None,
        (Some(commit_id), Some(record_sha256)) => {
            Some(PublicationToken::new(commit_id, record_sha256)?)
        }
        _ => return Err("the record names half a parent".to_string()),
    };
    if named_parent.as_ref() != parent {
        return Err("the record names another parent".to_string());
    }
    RecordName::new(
        fields.revision,
        PublicationToken::new(fields.commit_id, sha256_hex(bytes))?,
        named_parent,
        fields.created_at_ms,
    )
}

/// What one listing of a stream holds: every record name, and the chain
/// selected from them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamView {
    names: Vec<RecordName>,
    mainline: Mainline,
}

impl StreamView {
    fn from_names(names: Vec<RecordName>) -> Self {
        let mainline = Mainline::select(names.iter().cloned());
        Self { names, mainline }
    }

    pub fn mainline(&self) -> &Mainline {
        &self.mainline
    }

    pub fn into_mainline(self) -> Mainline {
        self.mainline
    }

    /// The selected chain, and the listed records that are kept and not
    /// selected.
    pub fn into_selection(self) -> (Mainline, Vec<RecordName>) {
        let mut unselected = self
            .names
            .into_iter()
            .filter(|name| !self.mainline.contains_token(name.token()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        unselected.sort_by_key(RecordName::file_name);
        (self.mainline, unselected)
    }

    /// Whether the listing holds this publication, selected or not.
    pub fn holds(&self, token: &PublicationToken) -> bool {
        self.names.iter().any(|name| name.token() == token)
    }

    /// Whether `name` may be written now: its parent is the selected
    /// publication, so the parent has no child. A first publication names no
    /// parent, so it needs a stream without a selected chain. A listed record
    /// that connects to no chain does not count against it: a Push whose
    /// parent was cleared under it leaves one behind and fails as a whole,
    /// and the cleared stream must still take its next first version.
    fn admits(&self, name: &RecordName) -> bool {
        self.mainline.selected().map(RecordName::token) == name.parent()
    }
}

#[derive(Default)]
struct StreamMemory {
    seen: HashSet<RecordName>,
    records: HashMap<PublicationToken, Vec<u8>>,
}

/// The process memory of one store's streams: the names of each stream's
/// last listing and the verified records read from it. Records are immutable
/// and named by their digest, so an entry never goes stale.
#[derive(Default)]
pub struct PublicationCache {
    streams: Mutex<HashMap<String, StreamMemory>>,
}

impl PublicationCache {
    fn with_stream<T>(
        &self,
        stream: &RecordStream,
        read: impl FnOnce(&mut StreamMemory) -> T,
    ) -> Option<T> {
        let mut streams = self.streams.lock().ok()?;
        Some(read(
            streams
                .entry(stream.prefix.as_str().to_string())
                .or_default(),
        ))
    }

    fn record(&self, stream: &RecordStream, token: &PublicationToken) -> Option<Vec<u8>> {
        self.with_stream(stream, |memory| memory.records.get(token).cloned())
            .flatten()
    }

    fn remember_record(&self, stream: &RecordStream, token: &PublicationToken, bytes: &[u8]) {
        self.with_stream(stream, |memory| {
            if memory.records.len() >= MAX_CACHED_RECORDS_PER_STREAM {
                memory.records.clear();
            }
            memory.records.insert(token.clone(), bytes.to_vec());
        });
    }
}

fn list_record_keys<S: ObjectStore + ?Sized>(
    store: &S,
    prefix: &ObjectPrefix,
) -> Result<Vec<ObjectKey>, String> {
    let mut keys = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        // A failed page fails the listing. It is never an empty stream.
        let page = store.list_keys(prefix, cursor.as_deref(), MAX_LIST_PAGE)?;
        keys.extend(page.keys);
        if keys.len() > MAX_LISTED_RECORDS {
            return Err(format!(
                "the publication listing of '{}' exceeds the {}-record limit; the view is incomplete",
                prefix.as_str(),
                MAX_LISTED_RECORDS
            ));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(keys),
        }
    }
}

/// The record name a listed key states, when the key sits directly in
/// `directory` and its name is within the grammar.
fn record_name_in(directory: &str, key: &ObjectKey) -> Option<RecordName> {
    let name = key
        .as_str()
        .strip_prefix(directory)?
        .strip_prefix('/')
        .filter(|name| !name.contains('/'))?;
    RecordName::parse(name)
}

/// List one stream and select. A publication this process has already seen
/// that is missing from the listing is confirmed with one direct read of its
/// key before it is dropped: if it reads, it stays, because the listing
/// lagged. Only a missing name whose parent is present is worth the read;
/// anything below a vanished record is disconnected either way.
pub fn list_stream<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
) -> Result<StreamView, String> {
    list_stream_at(store, stream, &SystemTime::now).map(|(view, _)| view)
}

/// `list_stream`, with the wall-clock time at which the listing answered:
/// taken before any confirmation read, because the freshness of a listing is
/// measured from the listing itself.
fn list_stream_at<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    wall_clock: &dyn Fn() -> SystemTime,
) -> Result<(StreamView, SystemTime), String> {
    let keys = list_record_keys(store, &stream.prefix)?;
    let listed_at = wall_clock();
    let mut names = Vec::new();
    for key in keys {
        match record_name_in(stream.prefix.as_str(), &key) {
            Some(name) => names.push(name),
            None => report_ignored_name(key.as_str()),
        }
    }
    let Some(cache) = store.publication_cache() else {
        return Ok((StreamView::from_names(names), listed_at));
    };
    let listed = names.iter().cloned().collect::<HashSet<_>>();
    let mut missing = cache
        .with_stream(stream, |memory| {
            memory.seen.difference(&listed).cloned().collect::<Vec<_>>()
        })
        .unwrap_or_default();
    missing.sort_by_key(RecordName::revision);
    let mut present = listed
        .iter()
        .map(|name| name.token().clone())
        .collect::<HashSet<_>>();
    for name in missing {
        if name
            .parent()
            .is_some_and(|parent| !present.contains(parent))
        {
            continue;
        }
        if store
            .get_metadata(&stream.key(&name)?, MAX_RECORD_BYTES)?
            .is_some()
        {
            present.insert(name.token().clone());
            names.push(name);
        }
    }
    cache.with_stream(stream, |memory| {
        memory.seen = names.iter().cloned().collect();
        let seen = &memory.seen;
        memory
            .records
            .retain(|token, _| seen.iter().any(|name| name.token() == token));
    });
    Ok((StreamView::from_names(names), listed_at))
}

/// List every project stream in one pass over the flat record namespace and
/// select each. No payload key is listed, and no record is read.
pub fn list_project_streams<S: ObjectStore + ?Sized>(
    store: &S,
) -> Result<Vec<(BundleId, Mainline)>, String> {
    let prefix = ObjectPrefix::parse(REPOSITORY_COMMITS_PREFIX)?;
    let mut streams = HashMap::<String, Vec<RecordName>>::new();
    for key in list_record_keys(store, &prefix)? {
        let parsed = key
            .as_str()
            .strip_prefix(REPOSITORY_COMMITS_PREFIX)
            .and_then(|rest| rest.strip_prefix('/'))
            .and_then(|rest| rest.split_once('/'))
            .filter(|(_, name)| !name.contains('/'))
            .and_then(|(bundle, name)| {
                let bundle = BundleId::parse(bundle.to_string()).ok()?;
                Some((bundle, RecordName::parse(name)?))
            });
        match parsed {
            Some((bundle, name)) => streams.entry(bundle.to_string()).or_default().push(name),
            None => report_ignored_name(key.as_str()),
        }
    }
    let mut selected = streams
        .into_iter()
        .map(|(bundle, names)| Ok((BundleId::parse(bundle)?, Mainline::select(names))))
        .collect::<Result<Vec<_>, String>>()?;
    selected.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(selected)
}

fn verify_record(name: &RecordName, key: &ObjectKey, bytes: &[u8]) -> Result<(), String> {
    if sha256_hex(bytes) != name.token().record_sha256() {
        return Err(format!(
            "publication record '{}' does not match the digest in its name",
            key
        ));
    }
    Ok(())
}

/// Read several records' exact bytes; results line up with `requests`. Every
/// record is verified against the digest in its name. A record that cannot
/// be read, or whose bytes do not hash to its name, is an error for that
/// publication; selection never moves to another candidate because of it.
pub fn read_records<S: ObjectStore + ?Sized>(
    store: &S,
    requests: &[(RecordStream, RecordName)],
) -> Result<Vec<Vec<u8>>, String> {
    let cache = store.publication_cache();
    let mut records = requests
        .iter()
        .map(|(stream, name)| cache.and_then(|cache| cache.record(stream, name.token())))
        .collect::<Vec<_>>();
    let pending = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.is_none())
        .map(|(index, _)| Ok((index, requests[index].0.key(&requests[index].1)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let keys = pending
        .iter()
        .map(|(_, key)| key.clone())
        .collect::<Vec<_>>();
    let fetched = store.get_metadata_many(&keys, MAX_RECORD_BYTES)?;
    if fetched.len() != pending.len() {
        return Err("publication record read returned the wrong result count".to_string());
    }
    for ((index, key), stored) in pending.into_iter().zip(fetched) {
        let (stream, name) = &requests[index];
        let stored = stored.ok_or_else(|| format!("publication record '{}' is missing", key))?;
        verify_record(name, &key, &stored.bytes)?;
        if let Some(cache) = cache {
            cache.remember_record(stream, name.token(), &stored.bytes);
        }
        records[index] = Some(stored.bytes);
    }
    Ok(records
        .into_iter()
        .map(|record| record.expect("every pending record was filled"))
        .collect())
}

pub fn read_record<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
) -> Result<Vec<u8>, String> {
    read_records(store, &[(stream.clone(), name.clone())])?
        .pop()
        .ok_or_else(|| "publication record read returned nothing".to_string())
}

/// Write one record with list-then-write and return the listing that selects
/// it. Every attempt is the second half of one step: list the stream, go on
/// only if the record's parent is selected and childless, claim the parent
/// on a store with claims, send the record once. A retry is a new attempt
/// and lists first, never a resend; nothing is kept between attempts or
/// across operations.
///
/// A record is published only when the listing after its write selects it.
/// On a store without claims two computers can both list the same childless
/// parent and both write; the one whose record ranks second has failed, and
/// its record is kept in the storage and never selected. On a store with
/// claims the parent's claim admits one child: the second computer finds
/// the claim taken and stops before it writes.
///
/// The claim and the record are sent once each. A request the store does not
/// acknowledge is settled by one read-only check, the claim key or a listing:
/// what it shows landed counts, what it shows absent did not land and the
/// push fails with the store's own error, and a check that cannot be made is
/// [`OUTCOME_UNKNOWN`]. Nothing is sent again; the next push is the user's.
///
/// Errors: [`REMOTE_CHANGED`] when the listing before the write shows another
/// child or another selection, when the claim is taken by a record a listing
/// holds, and when the listing that holds the record selects another
/// computer's, all with nothing of this record written. A claim taken by a
/// record no listing holds is [`WriteOutcome::Unfinished`]: nothing is
/// written, and the caller decides (section 7.3 of the CAS-free plan).
pub fn write_record<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
    bytes: &[u8],
) -> Result<WriteOutcome, String> {
    write_record_at(store, stream, name, bytes, &SystemTime::now)
}

/// Write the record an earlier push claimed and left unwritten, as this
/// operation's own: the claim already holds exactly these bytes, so no claim
/// is made, the record request follows, and the listing after it decides.
/// The caller has just found the claimed parent selected and childless.
pub fn write_claimed_record<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    claimed: &ClaimedRecord,
) -> Result<StreamView, String> {
    attempt(
        store,
        stream,
        &claimed.name,
        &claimed.bytes,
        &SystemTime::now,
        false,
    )?
    .stored()
}

fn listing_is_fresh(listed_at: SystemTime, now: SystemTime) -> bool {
    // A clock that ran backwards proves nothing about the listing's age.
    now.duration_since(listed_at)
        .is_ok_and(|age| age <= LISTING_FRESHNESS)
}

pub(crate) fn write_record_at<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
    bytes: &[u8],
    wall_clock: &dyn Fn() -> SystemTime,
) -> Result<WriteOutcome, String> {
    attempt(store, stream, name, bytes, wall_clock, true)
}

/// One claim request for the parent of `name`, carrying the record's bytes.
/// A request the store does not acknowledge is settled by one read of the
/// claim key: this computer's bytes and the claim is its own, other bytes
/// and it is taken, absent and the push fails with the store's error. A read
/// that cannot be made leaves the outcome unknown. Nothing is sent again.
fn claim_once<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
    bytes: &[u8],
) -> Result<ClaimOutcome, String> {
    let claim_key = stream.claim_key(name.parent())?;
    let refused = match store.put_claim(&claim_key, bytes) {
        Ok(outcome) => return Ok(outcome),
        Err(error) => error,
    };
    match store.get_metadata(&claim_key, MAX_RECORD_BYTES) {
        Ok(Some(found)) if found.bytes.as_slice() == bytes => Ok(ClaimOutcome::Mine),
        Ok(Some(found)) => Ok(ClaimOutcome::Taken(found.bytes)),
        Ok(None) => Err(refused),
        Err(_) => Err(OUTCOME_UNKNOWN.to_string()),
    }
}

/// The record `found` is, when it is a record of `parent`; anything else is
/// an integrity error for that parent, like a record whose bytes do not
/// match its name.
fn claimed_record_of(
    stream: &RecordStream,
    parent: Option<&PublicationToken>,
    found: &[u8],
) -> Result<RecordName, String> {
    record_name_of(found, parent).map_err(|reason| {
        format!(
            "the claim of {} in '{}' is not a record of it: {reason}",
            parent_label(parent),
            stream.prefix.as_str()
        )
    })
}

/// The claim of `name`'s parent holds another record. A record the listing
/// holds, or one fresh listing holds, is another computer's finished push:
/// the listing that admitted this record was older than that push. Otherwise
/// that push did not finish, and nothing writes its record by itself.
fn taken_claim<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
    view: &StreamView,
    found: Vec<u8>,
) -> Result<WriteOutcome, String> {
    let claimed = claimed_record_of(stream, name.parent(), &found)?;
    if view.holds(claimed.token()) || list_stream(store, stream)?.holds(claimed.token()) {
        return Err(REMOTE_CHANGED.to_string());
    }
    Ok(WriteOutcome::Unfinished(ClaimedRecord {
        name: claimed,
        bytes: found,
    }))
}

/// The listing selects the record: it is the publication, and the process
/// remembers it. Written is not published: a sibling that ranks first was
/// written in the same seconds, and this push failed as a whole.
fn stored<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
    bytes: &[u8],
    view: StreamView,
) -> Result<WriteOutcome, String> {
    if view.mainline.selected() != Some(name) {
        return Err(REMOTE_CHANGED.to_string());
    }
    if let Some(cache) = store.publication_cache() {
        cache.with_stream(stream, |memory| memory.seen.insert(name.clone()));
        cache.remember_record(stream, name.token(), bytes);
    }
    Ok(WriteOutcome::Stored(view))
}

/// One record request, settled by one listing: present and selected is
/// stored, present and not selected has failed, and absent did not land, so
/// the push fails with the store's error. A listing that cannot be made
/// leaves the outcome unknown. Nothing is sent again.
fn send_once<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    key: &ObjectKey,
    name: &RecordName,
    bytes: &[u8],
) -> Result<WriteOutcome, String> {
    let sent = store.put_record(key, bytes);
    let Ok(mut view) = list_stream(store, stream) else {
        return Err(OUTCOME_UNKNOWN.to_string());
    };
    if !view.holds(name.token()) {
        match sent {
            // The store acknowledged the record; a listing that lags behind
            // its own write does not unwrite it.
            Ok(_) => {
                let mut names = std::mem::take(&mut view.names);
                names.push(name.clone());
                view = StreamView::from_names(names);
            }
            Err(error) => return Err(error),
        }
    }
    stored(store, stream, name, bytes, view)
}

fn attempt<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
    name: &RecordName,
    bytes: &[u8],
    wall_clock: &dyn Fn() -> SystemTime,
    claim: bool,
) -> Result<WriteOutcome, String> {
    let key = stream.key(name)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("publication record exceeds its size limit".to_string());
    }
    verify_record(name, &key, bytes)?;

    // The parent is claimed once, on a store with claims, right before the
    // one record request. A record written for a claim this computer
    // already holds needs none.
    let claims = claim && store.supports_claims();
    for _ in 0..MAX_WRITE_LISTINGS {
        let (view, listed_at) = list_stream_at(store, stream, wall_clock)?;
        if view.holds(name.token()) {
            // An earlier attempt of this record landed.
            return stored(store, stream, name, bytes, view);
        }
        if !view.admits(name) {
            return Err(REMOTE_CHANGED.to_string());
        }
        if !listing_is_fresh(listed_at, wall_clock()) {
            continue;
        }
        if claims {
            if let ClaimOutcome::Taken(found) = claim_once(store, stream, name, bytes)? {
                return taken_claim(store, stream, name, &view, found);
            }
        }
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_before_record_request();
        return send_once(store, stream, &key, name, bytes);
    }
    Err("the storage listing could not be kept fresh enough to push".to_string())
}

/// The user's verb for a push that did not finish: write the record that
/// the claim of the selected version holds, once, through the same record
/// path, and report. A claim that is absent, or whose record a listing
/// already holds, is a storage that changed since the review that reported
/// the unfinished push. Nothing deletes or reassigns a claim, and nothing
/// runs this by itself.
pub fn finish_claimed_push<S: ObjectStore + ?Sized>(
    store: &S,
    stream: &RecordStream,
) -> Result<StreamView, String> {
    if !store.supports_claims() {
        return Err("this storage holds no claims, so there is no push to finish".to_string());
    }
    let view = list_stream(store, stream)?;
    let parent = view.mainline().selected().map(|name| name.token().clone());
    let Some(found) = store.get_metadata(&stream.claim_key(parent.as_ref())?, MAX_RECORD_BYTES)?
    else {
        return Err(REMOTE_CHANGED.to_string());
    };
    let claimed = claimed_record_of(stream, parent.as_ref(), &found.bytes)?;
    if view.holds(claimed.token()) {
        return Err(REMOTE_CHANGED.to_string());
    }
    write_claimed_record(
        store,
        stream,
        &ClaimedRecord {
            name: claimed,
            bytes: found.bytes,
        },
    )
}

/// Test control over the instant between a record's listing and its write,
/// for a test that reaches the store only through a whole service.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::cell::RefCell;

    thread_local! {
        static BEFORE_RECORD_REQUEST: RefCell<Option<Box<dyn FnOnce()>>> =
            const { RefCell::new(None) };
    }

    /// Run `action` once on this thread, after the next record has cleared
    /// its listing and right before it is sent: whatever another computer
    /// does in that instant.
    pub fn before_next_record_request(action: impl FnOnce() + 'static) {
        BEFORE_RECORD_REQUEST.with(|slot| *slot.borrow_mut() = Some(Box::new(action)));
    }

    pub(super) fn run_before_record_request() {
        // Taken before it runs, so an action that publishes does not recurse.
        let action = BEFORE_RECORD_REQUEST.with(|slot| slot.borrow_mut().take());
        if let Some(action) = action {
            action();
        }
    }
}

/// The publication sequence every store must run identically: the same
/// record set selects the same mainline on Local, S3/R2, Dropbox, and a
/// store with nothing but put, get and list.
#[cfg(test)]
pub(crate) mod parity {
    use super::*;

    /// The listing that selects the record; an unfinished push is an error.
    pub(crate) fn write_record<S: ObjectStore + ?Sized>(
        store: &S,
        stream: &RecordStream,
        name: &RecordName,
        bytes: &[u8],
    ) -> Result<StreamView, String> {
        super::write_record(store, stream, name, bytes).and_then(WriteOutcome::stored)
    }

    pub(crate) fn bundle(number: u64) -> BundleId {
        BundleId::parse(format!("{number:032x}")).unwrap()
    }

    pub(crate) fn record(
        parent: Option<&RecordName>,
        id: u8,
        created_at_ms: u64,
    ) -> (RecordName, Vec<u8>) {
        let bytes = format!("record {id} at {created_at_ms}").into_bytes();
        let token = PublicationToken::new(
            format!("commit-{}", format!("{id:02x}").repeat(12)),
            sha256_hex(&bytes),
        )
        .unwrap();
        let name = RecordName::new(
            parent.map_or(1, |parent| parent.revision() + 1),
            token,
            parent.map(|parent| parent.token().clone()),
            created_at_ms,
        )
        .unwrap();
        (name, bytes)
    }

    pub(crate) fn selected(view: &StreamView) -> Option<&RecordName> {
        view.mainline().selected()
    }

    /// The same publication sequence over every kind of store.
    pub(crate) fn publishes_and_selects(store: &dyn ObjectStore) {
        let stream = RecordStream::project(&bundle(1)).unwrap();
        let other = RecordStream::project(&bundle(2)).unwrap();
        assert_eq!(list_stream(store, &stream).unwrap(), StreamView::default());

        let (first, first_bytes) = record(None, 1, 100);
        let view = write_record(store, &stream, &first, &first_bytes).unwrap();
        assert_eq!(selected(&view), Some(&first));
        let (second, second_bytes) = record(Some(&first), 2, 200);
        let view = write_record(store, &stream, &second, &second_bytes).unwrap();
        assert_eq!(selected(&view), Some(&second));
        assert_eq!(read_record(store, &stream, &second).unwrap(), second_bytes);

        // A writer that lists a child under its parent stops; nothing is
        // written, however early its record claims to be.
        let (late, late_bytes) = record(Some(&first), 3, 1);
        assert_eq!(
            write_record(store, &stream, &late, &late_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        // A first publication needs a stream without a selected chain.
        let (rival, rival_bytes) = record(None, 4, 1);
        assert_eq!(
            write_record(store, &stream, &rival, &rival_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        assert!(!list_stream(store, &stream).unwrap().holds(late.token()));

        // Streams never compete: the other stream is still empty.
        let view = write_record(store, &other, &rival, &rival_bytes).unwrap();
        assert_eq!(selected(&view), Some(&rival));
        assert_eq!(
            selected(&list_stream(store, &stream).unwrap()),
            Some(&second)
        );

        // A repeat writes nothing new: the listing already holds the token.
        let view = write_record(store, &stream, &second, &second_bytes).unwrap();
        assert_eq!(selected(&view), Some(&second));

        let discovered = list_project_streams(store).unwrap();
        assert_eq!(
            discovered
                .iter()
                .map(|(id, mainline)| (id.clone(), mainline.selected().cloned()))
                .collect::<Vec<_>>(),
            vec![(bundle(1), Some(second)), (bundle(2), Some(rival))]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::plain_store::PlainStore;
    use super::super::{LocalObjectStore, ObjectStore};
    use super::parity::{bundle, publishes_and_selects, record, selected, write_record};
    use super::*;
    use std::cell::Cell;
    use std::time::Instant;

    #[test]
    fn the_local_store_and_a_plain_overwrite_store_run_the_same_publication() {
        let temp = tempfile::tempdir().unwrap();
        publishes_and_selects(&LocalObjectStore::open(temp.path().join("store")).unwrap());
        publishes_and_selects(&PlainStore::default());
    }

    #[test]
    fn two_writers_of_one_parent_are_both_kept_and_the_one_not_selected_has_failed() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(3)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        // Both writers list the same childless parent, so both records land:
        // the fork window. The earlier-ranked one lands between the later
        // writer's listing and its write.
        let (earlier, earlier_bytes) = record(Some(&root), 2, 200);
        let (later, later_bytes) = record(Some(&root), 3, 201);
        store.before_next_record_request({
            let (store, key) = (store.clone(), stream.key(&earlier).unwrap());
            let bytes = earlier_bytes.clone();
            move || {
                store.put_immutable(&key, &bytes).unwrap();
            }
        });
        store.reset_counts();
        // Its record was sent once and is in the storage, and the push failed.
        assert_eq!(
            write_record(&store, &stream, &later, &later_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        assert_eq!(store.record_requests(), 1);

        let view = list_stream(&store, &stream).unwrap();
        assert!(view.holds(earlier.token()) && view.holds(later.token()));
        assert_eq!(selected(&view), Some(&earlier));
        assert!(!view.mainline().contains_token(later.token()));
        // Asking again changes nothing and sends nothing: the record is
        // listed, and it is still not the selected one.
        store.reset_counts();
        assert_eq!(
            write_record(&store, &stream, &later, &later_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        assert_eq!(store.record_requests(), 0);
        // The writer whose record is selected reads it from the same call.
        let repeat = write_record(&store, &stream, &earlier, &earlier_bytes).unwrap();
        assert_eq!(selected(&repeat), Some(&earlier));
        // Nothing can be built on the record that was not selected.
        let (built_on_loser, bytes) = record(Some(&later), 4, 1);
        assert_eq!(
            write_record(&store, &stream, &built_on_loser, &bytes).unwrap_err(),
            REMOTE_CHANGED
        );
    }

    /// R4: two computers push the first version onto an empty stream in the
    /// same seconds. Both first records are kept, the earlier-ranked one is
    /// selected, and the other Push fails as a whole.
    #[test]
    fn two_first_publications_are_both_kept_and_the_later_one_has_failed() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(4)).unwrap();
        let (earlier, earlier_bytes) = record(None, 1, 100);
        let (later, later_bytes) = record(None, 2, 101);
        // Both writers list an empty stream. The earlier-ranked record lands
        // between the later writer's listing and its write.
        store.before_next_record_request({
            let (store, key) = (store.clone(), stream.key(&earlier).unwrap());
            let bytes = earlier_bytes.clone();
            move || {
                store.put_immutable(&key, &bytes).unwrap();
            }
        });
        assert_eq!(
            write_record(&store, &stream, &later, &later_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        let view = list_stream(&store, &stream).unwrap();
        assert!(view.holds(earlier.token()) && view.holds(later.token()));
        assert_eq!(selected(&view), Some(&earlier));
        assert!(!view.mainline().contains_token(later.token()));
        // The history goes on from the selected first version only.
        let (built_on_loser, bytes) = record(Some(&later), 3, 1);
        assert_eq!(
            write_record(&store, &stream, &built_on_loser, &bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        let (second, second_bytes) = record(Some(&earlier), 4, 200);
        let view = write_record(&store, &stream, &second, &second_bytes).unwrap();
        assert_eq!(selected(&view), Some(&second));
    }

    /// R5: another computer clears the storage while this one is pushing.
    /// The parent this Push listed is gone when its record lands, so the
    /// record connects to nothing, is never selected, and the Push fails as
    /// a whole. It never becomes the first version of the cleared stream.
    #[test]
    fn a_record_whose_parent_was_cleared_is_never_selected_and_its_push_fails() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(5)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        let (child, child_bytes) = record(Some(&root), 2, 200);
        // The clear lands between this writer's listing and its write.
        store.before_next_record_request({
            let store = store.clone();
            move || store.clear()
        });
        assert_eq!(
            write_record(&store, &stream, &child, &child_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        // The record is in the storage, and it is not a history.
        let view = list_stream(&store, &stream).unwrap();
        assert!(view.holds(child.token()));
        assert_eq!(selected(&view), None);
        assert!(view.mainline().is_empty());
        // The stream is still open for a first version, and the orphan stays
        // outside the chain that starts there.
        let (first, first_bytes) = record(None, 3, 300);
        let view = write_record(&store, &stream, &first, &first_bytes).unwrap();
        assert_eq!(selected(&view), Some(&first));
        assert!(!view.mainline().contains_token(child.token()));
    }

    #[test]
    fn a_listing_older_than_ten_wall_clock_seconds_is_listed_again() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(4)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        let (mine, mine_bytes) = record(Some(&root), 2, 200);
        let (theirs, their_bytes) = record(Some(&root), 3, 300);

        // The machine sleeps between the listing and the send: the wall
        // clock jumps a day while the monotonic clock, which stands still
        // during sleep on macOS, barely moves. Another computer publishes
        // on the same parent meanwhile.
        let started = Instant::now();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_789_635_601);
        let reads = Cell::new(0_u32);
        let wall_clock = || {
            reads.set(reads.get() + 1);
            if reads.get() == 2 {
                store
                    .put_record(&stream.key(&theirs).unwrap(), &their_bytes)
                    .unwrap();
                return start + Duration::from_secs(24 * 60 * 60);
            }
            start
        };
        store.reset_counts();
        let error = write_record_at(&store, &stream, &mine, &mine_bytes, &wall_clock).unwrap_err();
        assert_eq!(error, REMOTE_CHANGED);
        assert!(started.elapsed() < LISTING_FRESHNESS);
        // On wake it listed again, saw the child, and sent nothing of its own.
        assert_eq!(store.listings(), 2);
        assert!(!list_stream(&store, &stream).unwrap().holds(mine.token()));

        // Ten seconds exactly is still fresh; a clock that ran backwards is not.
        assert!(listing_is_fresh(start, start + LISTING_FRESHNESS));
        assert!(!listing_is_fresh(
            start,
            start + LISTING_FRESHNESS + Duration::from_millis(1)
        ));
        assert!(!listing_is_fresh(start, start - Duration::from_secs(1)));
    }

    /// The record is sent once. A request the store refuses is settled by
    /// the one listing that follows it: absent, the push fails with the
    /// store's error and nothing is sent again; present, the record landed
    /// and only its acknowledgment was lost. The next push is the user's.
    #[test]
    fn a_failed_record_request_is_never_resent() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(5)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        store.fail_record_requests(1, false);
        let error = write_record(&store, &stream, &root, &root_bytes).unwrap_err();
        assert!(error.contains("record request failed"), "{error}");
        assert_eq!((store.listings(), store.record_requests()), (2, 1));
        assert!(!list_stream(&store, &stream).unwrap().holds(root.token()));
        // The user's next push lists first and sends once.
        store.reset_counts();
        let view = write_record(&store, &stream, &root, &root_bytes).unwrap();
        assert_eq!(selected(&view), Some(&root));
        assert_eq!((store.listings(), store.record_requests()), (2, 1));

        // The acknowledgment is lost: the record landed, and the one
        // listing settles it.
        let (next, next_bytes) = record(Some(&root), 2, 200);
        store.reset_counts();
        store.fail_record_requests(1, true);
        let view = write_record(&store, &stream, &next, &next_bytes).unwrap();
        assert_eq!(selected(&view), Some(&next));
        assert_eq!((store.listings(), store.record_requests()), (2, 1));

        // A transport that keeps failing: still one request, nothing kept.
        let (third, third_bytes) = record(Some(&next), 3, 300);
        store.reset_counts();
        store.fail_record_requests(usize::MAX, false);
        let error = write_record(&store, &stream, &third, &third_bytes).unwrap_err();
        assert!(error.contains("record request failed"), "{error}");
        assert_eq!(store.record_requests(), 1);
        store.fail_record_requests(0, false);
        assert!(!list_stream(&store, &stream).unwrap().holds(third.token()));
    }

    #[test]
    fn an_unreachable_storage_after_sending_reports_an_unknown_outcome() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(6)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        // Before anything is sent, a failed listing is just that failure.
        store.fail_listings_after(0);
        let error = write_record(&store, &stream, &root, &root_bytes).unwrap_err();
        assert!(error.contains("listing failed"), "{error}");
        assert_eq!(store.record_requests(), 0);

        // After a record was sent, it is an unknown outcome.
        store.fail_listings_after(1);
        store.fail_record_requests(1, true);
        assert_eq!(
            write_record(&store, &stream, &root, &root_bytes).unwrap_err(),
            OUTCOME_UNKNOWN
        );
        // The next refresh shows that it arrived.
        store.fail_listings_after(usize::MAX);
        assert_eq!(
            selected(&list_stream(&store, &stream).unwrap()),
            Some(&root)
        );
    }

    #[test]
    fn a_record_that_does_not_hash_to_its_name_is_an_integrity_error_and_selection_stays() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(7)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        let (tip, tip_bytes) = record(Some(&root), 2, 200);
        assert!(write_record(&store, &stream, &tip, b"other bytes").is_err());
        store
            .put_record(&stream.key(&tip).unwrap(), b"other bytes")
            .unwrap();

        let view = list_stream(&store, &stream).unwrap();
        assert_eq!(selected(&view), Some(&tip));
        let error = read_record(&store, &stream, &tip).unwrap_err();
        assert!(
            error.contains("does not match the digest in its name"),
            "{error}"
        );
        assert_eq!(selected(&list_stream(&store, &stream).unwrap()), Some(&tip));
        let _ = tip_bytes;
    }

    #[test]
    fn foreign_names_are_ignored_and_reported_and_the_listing_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("store");
        let store = LocalObjectStore::open(&root).unwrap();
        let stream = RecordStream::project(&bundle(8)).unwrap();
        let (first, first_bytes) = record(None, 1, 100);
        write_record(&store, &stream, &first, &first_bytes).unwrap();
        let mut events = ignored_name_events().subscribe();
        let directory = root.join(stream.prefix.as_str());
        let conflicted = first
            .file_name()
            .replace(".json", " (conflicted copy).json");
        std::fs::write(directory.join(".DS_Store"), b"finder").unwrap();
        std::fs::write(directory.join(&conflicted), &first_bytes).unwrap();
        std::fs::create_dir(directory.join("nested")).unwrap();
        std::fs::write(directory.join("nested").join(first.file_name()), b"x").unwrap();

        let view = list_stream(&store, &stream).unwrap();
        assert_eq!(view.names, vec![first.clone()]);
        assert_eq!(
            list_project_streams(&store)
                .unwrap()
                .into_iter()
                .map(|(id, mainline)| (id, mainline.selected().cloned()))
                .collect::<Vec<_>>(),
            vec![(bundle(8), Some(first))]
        );
        let mut reported = Vec::new();
        while let Ok(name) = events.try_recv() {
            reported.push(name);
        }
        for foreign in [".DS_Store", conflicted.as_str(), "nested/"] {
            let key = format!("{}/{}", stream.prefix.as_str(), foreign);
            assert!(
                reported.iter().any(|name| name.starts_with(&key)),
                "{foreign} was not reported"
            );
        }
    }

    #[test]
    fn an_unchanged_selection_is_one_listing_and_a_lagging_listing_keeps_what_was_seen() {
        let store = PlainStore::default().with_publication_cache();
        let stream = RecordStream::project(&bundle(9)).unwrap();
        let (root, root_bytes) = record(None, 1, 100);
        let (tip, tip_bytes) = record(Some(&root), 2, 200);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        write_record(&store, &stream, &tip, &tip_bytes).unwrap();

        // The record this process wrote is already held: nothing is read.
        store.reset_counts();
        let view = list_stream(&store, &stream).unwrap();
        assert_eq!(selected(&view), Some(&tip));
        assert_eq!(read_record(&store, &stream, &tip).unwrap(), tip_bytes);
        assert_eq!((store.listings(), store.reads()), (1, 0));

        // A listing that lags behind a write this process has seen: one
        // direct read of the key confirms it, and it stays.
        store.hide_from_listings(&stream.key(&tip).unwrap());
        store.reset_counts();
        assert_eq!(selected(&list_stream(&store, &stream).unwrap()), Some(&tip));
        assert_eq!(store.reads(), 1);

        // A cleared stream: the first publication is confirmed gone with one
        // read, and nothing below it is worth another.
        store.clear();
        store.reset_counts();
        assert_eq!(list_stream(&store, &stream).unwrap(), StreamView::default());
        assert_eq!(store.reads(), 1);
        store.reset_counts();
        assert_eq!(list_stream(&store, &stream).unwrap(), StreamView::default());
        assert_eq!(store.reads(), 0);
    }

    #[test]
    fn a_listing_past_its_budget_is_an_incomplete_view_and_never_a_truncated_history() {
        let store = PlainStore::default();
        let stream = RecordStream::project(&bundle(10)).unwrap();
        for index in 0..=MAX_LISTED_RECORDS {
            let key = ObjectKey::parse(format!("{}/{index:06}", stream.prefix.as_str())).unwrap();
            store.put_immutable(&key, b"").unwrap();
        }
        let started = std::time::Instant::now();
        let error = list_stream(&store, &stream).unwrap_err();
        assert!(error.contains("the view is incomplete"), "{error}");
        // A push cannot go on from an incomplete view either.
        let (first, first_bytes) = record(None, 1, 100);
        assert!(write_record(&store, &stream, &first, &first_bytes)
            .unwrap_err()
            .contains("the view is incomplete"));
        assert_eq!(store.record_requests(), 0);
        eprintln!(
            "listing {} names to the budget took {:?} in memory",
            MAX_LISTED_RECORDS,
            started.elapsed()
        );
    }

    #[test]
    fn library_streams_live_inside_their_repository() {
        let repository = format!("library-{}", "a".repeat(32));
        let stream = RecordStream::library(&repository).unwrap();
        let (first, _) = record(None, 1, 100);
        assert_eq!(
            stream.key(&first).unwrap().as_str(),
            format!(
                ".mallard/v1/library/{repository}/_commits/{}",
                first.file_name()
            )
        );
        assert!(RecordStream::library("catalog-nope").is_err());
        assert_eq!(
            RecordStream::project(&bundle(1))
                .unwrap()
                .key(&first)
                .unwrap()
                .as_str(),
            format!(
                ".mallard/v1/repositories/_commits/{:032x}/{}",
                1,
                first.file_name()
            )
        );
    }

    /// The plain store with claims: the least a store with claims has to be.
    fn claiming() -> PlainStore {
        PlainStore::default().with_claims()
    }

    /// A record whose bytes carry the five graph facts, as both engines'
    /// records do, so a claim of it names it.
    fn json_record(
        parent: Option<&RecordName>,
        id: u8,
        created_at_ms: u64,
    ) -> (RecordName, Vec<u8>) {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "revision": parent.map_or(1, |parent| parent.revision() + 1),
            "commit_id": format!("commit-{}", format!("{id:02x}").repeat(12)),
            "previous_commit_id": parent.map(RecordName::commit_id),
            "previous_record_sha256": parent.map(|parent| parent.token().record_sha256()),
            "created_at_ms": created_at_ms,
            "note": format!("record {id}"),
        }))
        .unwrap();
        let name = record_name_of(&bytes, parent.map(RecordName::token)).unwrap();
        (name, bytes)
    }

    fn claim_bytes(
        store: &PlainStore,
        stream: &RecordStream,
        parent: Option<&PublicationToken>,
    ) -> Option<Vec<u8>> {
        store
            .get_metadata(&stream.claim_key(parent).unwrap(), MAX_RECORD_BYTES)
            .unwrap()
            .map(|found| found.bytes)
    }

    fn counts(store: &PlainStore) -> (usize, usize, usize) {
        (
            store.listings(),
            store.claim_requests(),
            store.record_requests(),
        )
    }

    /// Two Pushes onto one version in the same seconds. The claim decides,
    /// not the rank: the second computer's claim is refused in one request,
    /// its record is never written, and the version has one child. Its
    /// listing predated the other computer's record, so the taken path
    /// lists once more and reads a finished push, never an unfinished one.
    #[test]
    fn a_claim_admits_one_child_per_parent_and_a_stale_listing_ends_in_remote_changed() {
        let store = claiming();
        let stream = RecordStream::project(&bundle(20)).unwrap();
        let (a, a_bytes) = json_record(None, 1, 100);
        let (b, b_bytes) = json_record(None, 2, 99);
        // Two first versions: B lists an empty stream, A claims the root and
        // writes its record before B's claim. B's record would rank first.
        store.before_next_claim_request({
            let (store, stream) = (store.clone(), stream.clone());
            let (a, a_bytes) = (a.clone(), a_bytes.clone());
            move || {
                write_record(&store, &stream, &a, &a_bytes).unwrap();
            }
        });
        store.reset_counts();
        assert_eq!(
            write_record(&store, &stream, &b, &b_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        // B: its listing, its refused claim, one fresh listing. A, inside the
        // hook: a listing, a claim, a record, a listing.
        assert_eq!(counts(&store), (4, 2, 1));
        let view = list_stream(&store, &stream).unwrap();
        assert!(view.holds(a.token()) && !view.holds(b.token()));
        assert_eq!(selected(&view), Some(&a));
        assert_eq!(claim_bytes(&store, &stream, None), Some(a_bytes));

        // The same on a version.
        let (a2, a2_bytes) = json_record(Some(&a), 3, 300);
        let (b2, b2_bytes) = json_record(Some(&a), 4, 299);
        store.before_next_claim_request({
            let (store, stream) = (store.clone(), stream.clone());
            let (a2, a2_bytes) = (a2.clone(), a2_bytes.clone());
            move || {
                write_record(&store, &stream, &a2, &a2_bytes).unwrap();
            }
        });
        assert_eq!(
            write_record(&store, &stream, &b2, &b2_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        let view = list_stream(&store, &stream).unwrap();
        assert!(view.holds(a2.token()) && !view.holds(b2.token()));
        assert_eq!(selected(&view), Some(&a2));
        assert_eq!(
            claim_bytes(&store, &stream, Some(a.token())),
            Some(a2_bytes)
        );

        // A writer whose listing shows the child stops before any claim.
        store.reset_counts();
        let (late, late_bytes) = json_record(Some(&a), 5, 1);
        assert_eq!(
            write_record(&store, &stream, &late, &late_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        assert_eq!(counts(&store), (1, 0, 0));
    }

    /// A writer that claimed a version and died before its record. The next
    /// Push onto that version, from another computer, writes nothing and
    /// reports the unfinished push: one claim request, no record request,
    /// and the claimed record by name. Nothing finishes it by itself.
    #[test]
    fn a_writer_that_died_between_its_claim_and_its_record_leaves_a_push_nobody_finishes_by_itself()
    {
        let store = claiming();
        let stream = RecordStream::project(&bundle(21)).unwrap();
        let (root, root_bytes) = json_record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        let (a, a_bytes) = json_record(Some(&root), 2, 200);
        assert_eq!(
            store
                .put_claim(&stream.claim_key(Some(root.token())).unwrap(), &a_bytes)
                .unwrap(),
            ClaimOutcome::Mine
        );

        let (b, b_bytes) = json_record(Some(&root), 3, 300);
        store.reset_counts();
        let WriteOutcome::Unfinished(claimed) =
            super::write_record(&store, &stream, &b, &b_bytes).unwrap()
        else {
            panic!("a push onto a claimed version is unfinished")
        };
        assert_eq!(
            claimed,
            ClaimedRecord {
                name: a.clone(),
                bytes: a_bytes.clone()
            }
        );
        assert_eq!(counts(&store), (2, 1, 0));
        let view = list_stream(&store, &stream).unwrap();
        assert!(!view.holds(a.token()) && !view.holds(b.token()));
        assert_eq!(selected(&view), Some(&root));
        assert_eq!(
            WriteOutcome::Unfinished(claimed.clone())
                .stored()
                .unwrap_err(),
            PUSH_UNFINISHED
        );
        // Asking again changes nothing.
        assert!(matches!(
            super::write_record(&store, &stream, &b, &b_bytes).unwrap(),
            WriteOutcome::Unfinished(_)
        ));
        assert_eq!(store.record_requests(), 0);

        // Finishing it writes the claimed record, once, with no claim: it
        // is then the version's one child, and B's record has no place.
        store.reset_counts();
        let view = write_claimed_record(&store, &stream, &claimed).unwrap();
        assert_eq!(selected(&view), Some(&a));
        assert_eq!(counts(&store), (2, 0, 1));
        assert_eq!(
            write_record(&store, &stream, &b, &b_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
    }

    #[test]
    fn a_claim_that_is_not_a_record_of_its_parent_is_an_integrity_error_and_selection_stays() {
        let store = claiming();
        let stream = RecordStream::project(&bundle(22)).unwrap();
        let (root, root_bytes) = json_record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        let (child, child_bytes) = json_record(Some(&root), 2, 200);
        let claim = stream.claim_key(Some(root.token())).unwrap();
        store.reset_counts();

        store.put_immutable(&claim, b"not a record").unwrap();
        let error = write_record(&store, &stream, &child, &child_bytes).unwrap_err();
        assert!(error.contains("is not a record of it"), "{error}");
        assert!(error.contains("not a publication record"), "{error}");
        // A record of another parent: the root has no parent at all.
        store.put_immutable(&claim, &root_bytes).unwrap();
        let error = write_record(&store, &stream, &child, &child_bytes).unwrap_err();
        assert!(error.contains("names another parent"), "{error}");
        assert_eq!(store.record_requests(), 0);
        assert_eq!(
            selected(&list_stream(&store, &stream).unwrap()),
            Some(&root)
        );

        // A record's name is read from its bytes: the five facts and the
        // digest of the bytes themselves.
        assert_eq!(
            record_name_of(&child_bytes, Some(root.token())).unwrap(),
            child
        );
        assert!(record_name_of(&child_bytes, None).is_err());
        assert!(record_name_of(&[0_u8; 64 * 1024 + 1], None).is_err());
    }

    /// The claim proves that the parent has no other child, not that the
    /// parent is still there: a clear between the claim and the record
    /// leaves the record an orphan, and the listing after the write says
    /// so. The clear also freed the root's claim for the next first version.
    #[test]
    fn a_cleared_parent_leaves_the_claimed_record_an_orphan_and_the_root_free_to_claim_again() {
        let store = claiming();
        let stream = RecordStream::project(&bundle(23)).unwrap();
        let (root, root_bytes) = json_record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        let (child, child_bytes) = json_record(Some(&root), 2, 200);
        store.before_next_record_request({
            let store = store.clone();
            move || store.clear()
        });
        assert_eq!(
            write_record(&store, &stream, &child, &child_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        let view = list_stream(&store, &stream).unwrap();
        assert!(view.holds(child.token()));
        assert_eq!(selected(&view), None);
        assert_eq!(claim_bytes(&store, &stream, None), None);

        let (first, first_bytes) = json_record(None, 3, 300);
        let view = write_record(&store, &stream, &first, &first_bytes).unwrap();
        assert_eq!(selected(&view), Some(&first));
        assert!(!view.mainline().contains_token(child.token()));
        assert_eq!(claim_bytes(&store, &stream, None), Some(first_bytes));
    }

    /// A clear interrupted after its claims pass leaves the chain without
    /// its claims. A writer that listed the tip's parent childless before
    /// the tip was visible, and paused since, claims the parent again and
    /// lands an earlier-stamped sibling: the tip's computer reads Displaced
    /// at its next refresh, and nothing rolls back.
    #[test]
    fn an_interrupted_clear_that_removed_the_claims_lets_a_delayed_writer_displace_the_tip() {
        let store = claiming();
        let stream = RecordStream::project(&bundle(24)).unwrap();
        let (root, root_bytes) = json_record(None, 1, 100);
        let (tip, tip_bytes) = json_record(Some(&root), 2, 300);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        write_record(&store, &stream, &tip, &tip_bytes).unwrap();
        let (delayed, delayed_bytes) = json_record(Some(&root), 3, 200);
        let tip_key = stream.key(&tip).unwrap();

        // With the claims intact the delayed writer is refused: its own
        // listing predates the tip, and the fresh listing of the taken path
        // holds it.
        store.hide_from_listings(&tip_key);
        store.before_next_claim_request({
            let (store, tip_key) = (store.clone(), tip_key.clone());
            move || store.reveal_in_listings(&tip_key)
        });
        assert_eq!(
            write_record(&store, &stream, &delayed, &delayed_bytes).unwrap_err(),
            REMOTE_CHANGED
        );
        assert!(!list_stream(&store, &stream).unwrap().holds(delayed.token()));

        // Without them it lands, and is selected.
        store.remove(&stream.claim_key(None).unwrap());
        store.remove(&stream.claim_key(Some(root.token())).unwrap());
        store.hide_from_listings(&tip_key);
        let view = write_record(&store, &stream, &delayed, &delayed_bytes).unwrap();
        assert_eq!(selected(&view), Some(&delayed));
        store.reveal_in_listings(&tip_key);
        let (mainline, unselected) = list_stream(&store, &stream).unwrap().into_selection();
        assert_eq!(mainline.selected(), Some(&delayed));
        assert_eq!(mainline.outranked(&unselected, tip.commit_id()), Some(&tip));
    }

    #[test]
    fn a_store_with_claims_costs_one_request_more_and_the_local_store_writes_none() {
        let store = claiming();
        publishes_and_selects(&store);
        let stream = RecordStream::project(&bundle(25)).unwrap();
        let (root, root_bytes) = json_record(None, 1, 100);
        store.reset_counts();
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        // Listing, claim, record, listing.
        assert_eq!(counts(&store), (2, 1, 1));
        // A repeat writes and claims nothing: the listing holds the token.
        store.reset_counts();
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        assert_eq!(counts(&store), (1, 0, 0));

        // The record request fails after the claim: the claim stays, and
        // the next write of the same record meets it as its own.
        let (next, next_bytes) = json_record(Some(&root), 2, 200);
        store.reset_counts();
        store.fail_record_requests(usize::MAX, false);
        let error = write_record(&store, &stream, &next, &next_bytes).unwrap_err();
        assert!(error.contains("record request failed"), "{error}");
        assert_eq!(store.claim_requests(), 1);
        assert_eq!(
            claim_bytes(&store, &stream, Some(root.token())),
            Some(next_bytes.clone())
        );
        store.fail_record_requests(0, false);
        store.reset_counts();
        let view = write_record(&store, &stream, &next, &next_bytes).unwrap();
        assert_eq!(selected(&view), Some(&next));
        assert_eq!(counts(&store), (2, 1, 1));

        // The local store writes no claim: its folder could be one a sync
        // product shares, and a network share promises no create-only write.
        let temp = tempfile::tempdir().unwrap();
        let local = LocalObjectStore::open(temp.path().join("store")).unwrap();
        assert!(!local.supports_claims());
        publishes_and_selects(&local);
        assert!(!temp
            .path()
            .join("store/.mallard/v1/repositories/_claims")
            .exists());
    }

    #[test]
    fn the_unfinished_push_sentence_names_the_storage() {
        assert_eq!(
            unfinished_push_sentence("Attic R2", PUSH_UNFINISHED.to_string()),
            "A push to Attic R2 did not finish. Finish it, then compare again."
        );
        assert_eq!(
            unfinished_push_sentence("Attic R2", OWN_PUSH_UNFINISHED.to_string()),
            "This computer's earlier push to Attic R2 did not finish. Finish it, then compare again."
        );
        assert_eq!(
            unfinished_push_sentence("Attic R2", REMOTE_CHANGED.to_string()),
            REMOTE_CHANGED
        );
    }

    /// The claim is sent once. A request the store does not acknowledge is
    /// settled by one read of the claim key, and never sent again.
    #[test]
    fn an_unacknowledged_claim_is_settled_by_one_read_and_never_sent_again() {
        let store = claiming();
        let stream = RecordStream::project(&bundle(26)).unwrap();
        let (root, root_bytes) = json_record(None, 1, 100);

        // The claim landed and its acknowledgment was lost: the read finds
        // this computer's bytes, and the push goes on to its record.
        store.fail_claim_requests(1, true);
        store.reset_counts();
        let view = write_record(&store, &stream, &root, &root_bytes).unwrap();
        assert_eq!(selected(&view), Some(&root));
        assert_eq!(counts(&store), (2, 1, 1));
        assert_eq!(store.reads(), 1);

        // The claim did not land: the push fails with the store's error.
        let (next, next_bytes) = json_record(Some(&root), 2, 200);
        store.fail_claim_requests(1, false);
        store.reset_counts();
        let error = write_record(&store, &stream, &next, &next_bytes).unwrap_err();
        assert!(error.contains("claim request failed"), "{error}");
        assert_eq!(counts(&store), (1, 1, 0));
        assert_eq!(claim_bytes(&store, &stream, Some(root.token())), None);

        // Another computer's claim is there: the read tells it is taken.
        let (theirs, their_bytes) = json_record(Some(&root), 3, 150);
        store
            .put_claim(&stream.claim_key(Some(root.token())).unwrap(), &their_bytes)
            .unwrap();
        store.fail_claim_requests(1, false);
        store.reset_counts();
        assert!(matches!(
            super::write_record(&store, &stream, &next, &next_bytes).unwrap(),
            WriteOutcome::Unfinished(ref claimed) if claimed.name == theirs
        ));
        assert_eq!(counts(&store), (2, 1, 0));

        // The read itself fails: the outcome is unknown, and nothing is
        // sent again. The claim landed after all, and the next push meets
        // it as this computer's own.
        store.remove(&stream.claim_key(Some(root.token())).unwrap());
        store.fail_claim_requests(1, true);
        store.fail_reads(1);
        store.reset_counts();
        assert_eq!(
            write_record(&store, &stream, &next, &next_bytes).unwrap_err(),
            OUTCOME_UNKNOWN
        );
        assert_eq!(counts(&store), (1, 1, 0));
        store.reset_counts();
        let view = write_record(&store, &stream, &next, &next_bytes).unwrap();
        assert_eq!(selected(&view), Some(&next));
        assert_eq!(counts(&store), (2, 1, 1));
    }

    /// The user's verb for a push that did not finish: it writes the claimed
    /// record once and reports. A claim that is absent, or whose record is
    /// listed, is a storage that changed; nothing runs the verb by itself.
    #[test]
    fn finishing_an_unfinished_push_writes_the_claimed_record_once_and_only_when_asked() {
        let store = claiming();
        let stream = RecordStream::project(&bundle(27)).unwrap();
        // Nothing to finish on an empty stream, or on a version nobody claimed.
        assert_eq!(
            finish_claimed_push(&store, &stream).unwrap_err(),
            REMOTE_CHANGED
        );
        let (root, root_bytes) = json_record(None, 1, 100);
        write_record(&store, &stream, &root, &root_bytes).unwrap();
        assert_eq!(
            finish_claimed_push(&store, &stream).unwrap_err(),
            REMOTE_CHANGED
        );

        // A writer died between its claim and its record.
        let (dead, dead_bytes) = json_record(Some(&root), 2, 200);
        store
            .put_claim(&stream.claim_key(Some(root.token())).unwrap(), &dead_bytes)
            .unwrap();
        store.reset_counts();
        let view = finish_claimed_push(&store, &stream).unwrap();
        assert_eq!(selected(&view), Some(&dead));
        // The listing that finds the claim, the one before the record, and
        // the one that settles it; no claim request.
        assert_eq!(counts(&store), (3, 0, 1));
        store.reset_counts();
        assert_eq!(
            finish_claimed_push(&store, &stream).unwrap_err(),
            REMOTE_CHANGED
        );
        assert_eq!(counts(&store), (1, 0, 0));

        // A refused record request: the store's error, and the claim stays
        // for another try.
        let (next_dead, next_dead_bytes) = json_record(Some(&dead), 3, 300);
        store
            .put_claim(
                &stream.claim_key(Some(dead.token())).unwrap(),
                &next_dead_bytes,
            )
            .unwrap();
        store.fail_record_requests(1, false);
        store.reset_counts();
        let error = finish_claimed_push(&store, &stream).unwrap_err();
        assert!(error.contains("record request failed"), "{error}");
        assert_eq!(store.record_requests(), 1);
        assert_eq!(
            claim_bytes(&store, &stream, Some(dead.token())),
            Some(next_dead_bytes)
        );
        assert_eq!(
            selected(&finish_claimed_push(&store, &stream).unwrap()),
            Some(&next_dead)
        );

        // A store without claims has nothing to finish, and a claim that is
        // not a record of the version is an integrity error.
        let error = finish_claimed_push(&PlainStore::default(), &stream).unwrap_err();
        assert!(error.contains("holds no claims"), "{error}");
        store
            .put_immutable(
                &stream.claim_key(Some(next_dead.token())).unwrap(),
                b"garbage",
            )
            .unwrap();
        let error = finish_claimed_push(&store, &stream).unwrap_err();
        assert!(error.contains("is not a record of it"), "{error}");
    }
}
