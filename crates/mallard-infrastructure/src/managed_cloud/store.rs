//! The managed store: the bundle engine's object-store contract answered by
//! the Mallard Cloud service. The storage marker is synthesized; objects are
//! staged in memory until the manifest arrives, which turns into the upload;
//! the record write is the commit.
//!
//! Mallard Cloud is not offered by this build. The engine reads by listing a
//! stream of publication records; the service stores no records and lists
//! none yet, so this store is not in the production registry
//! (DESIGN_CAS_FREE_STORAGE_REFACTOR.md, section 14).
//!
//! | Engine key | Behaviour |
//! | --- | --- |
//! | `.mallard/_storage.json` | synthesized marker |
//! | `_head.json` read | the repository status read: exact repository head when pinned, otherwise by bundle |
//! | `_manifests/{rev}-{commit}.json` read | streamed manifest, digest checked |
//! | `_objects/{sha}` read | streamed object, digest checked |
//! | `_objects/{sha}` write | staged in memory |
//! | `_manifests/…` write | create the upload, put the missing objects |
//! | `_commits/{bundle}/{record}` write | commit the upload; the service refuses a parent that is not its head |
//! | `_commits/…` read and list | absent and refused: the service stores no records yet |
//! | `_uploads/…` | refused: probes are for user-owned stores |
//! | `.mallard/v1/library/…` | reads absent; writes refused |
use super::client::{
    CreateRepository, Error, ErrorKind, InventoryEntry, ManagedCloudClient, UploadRequest,
};
use super::views::build_views_for_manifest;
use crate::providers::AgentProviderRegistry;
use crate::storage_core::publication::REMOTE_CHANGED;
use crate::storage_core::{
    ConnectionHealth, ExpectedObject, ImmutablePutOutcome, ObjectKey, ObjectPrefix, ObjectStore,
    StageOutcome, StorageMarker, StoreListPage, StoredMetadata, VerifiedBody, LIBRARY_PREFIX,
    REPOSITORY_COMMITS_PREFIX, REPOSITORY_PREFIX, STORAGE_MARKER_KEY,
};
use bytes::Bytes;
use mallard_domain::{sha256_hex, BundleHead, BundleId, BundleKind, BundleManifest, BUNDLE_SCHEMA};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::runtime::Handle;

pub const CATALOG_REFUSAL: &str =
    "managed_catalog_uses_item_adapter: Library items on Mallard Cloud are synced through the catalog item adapter, not the storage namespace.";

#[derive(Debug, PartialEq, Eq)]
enum Key {
    Marker,
    Head(BundleId),
    Manifest {
        bundle_id: BundleId,
        revision: u64,
        commit_id: String,
    },
    Record(BundleId),
    Object {
        bundle_id: BundleId,
        sha256: String,
    },
    Upload(BundleId),
    Library,
}

fn classify(key: &ObjectKey) -> Result<Key, String> {
    let raw = key.as_str();
    if raw == STORAGE_MARKER_KEY {
        return Ok(Key::Marker);
    }
    if raw.starts_with(&format!("{LIBRARY_PREFIX}/")) {
        return Ok(Key::Library);
    }
    if let Some(rest) = raw.strip_prefix(&format!("{REPOSITORY_COMMITS_PREFIX}/")) {
        let (bundle, _) = rest
            .split_once('/')
            .ok_or_else(|| format!("unexpected record key '{raw}'"))?;
        return Ok(Key::Record(BundleId::parse(bundle.to_string())?));
    }
    let rest = raw
        .strip_prefix(&format!("{REPOSITORY_PREFIX}/"))
        .ok_or_else(|| format!("object key is outside the managed namespaces: '{raw}'"))?;
    let (bundle, relative) = rest
        .split_once('/')
        .ok_or_else(|| format!("unexpected repository key '{raw}'"))?;
    let bundle_id = BundleId::parse(bundle.to_string())?;
    if relative == "_head.json" {
        return Ok(Key::Head(bundle_id));
    }
    if let Some(name) = relative.strip_prefix("_manifests/") {
        let (revision, commit_id) = name
            .strip_suffix(".json")
            .and_then(|stem| stem.split_once('-'))
            .ok_or_else(|| format!("unexpected manifest key '{raw}'"))?;
        return Ok(Key::Manifest {
            bundle_id,
            revision: revision
                .parse()
                .map_err(|_| format!("unexpected manifest key '{raw}'"))?,
            commit_id: commit_id.to_owned(),
        });
    }
    if let Some(sha256) = relative.strip_prefix("_objects/") {
        mallard_domain::validate_sha256("managed object", sha256)?;
        return Ok(Key::Object {
            bundle_id,
            sha256: sha256.to_owned(),
        });
    }
    if relative.starts_with("_uploads/") {
        return Ok(Key::Upload(bundle_id));
    }
    Err(format!("unexpected repository key '{raw}'"))
}

/// The upload created for a bundle's next snapshot, waiting for the record
/// write that commits it.
#[derive(Clone, Debug)]
struct PendingUpload {
    repository_id: String,
    upload_id: String,
    commit_id: String,
    revision: u64,
}

struct Inner {
    client: ManagedCloudClient,
    runtime: Handle,
    account_id: String,
    /// Bundle → repository id, learned from heads, listings, and creates.
    repositories: Mutex<BTreeMap<BundleId, String>>,
    /// Exact repository destinations from link metadata.
    pinned: Mutex<BTreeMap<BundleId, String>>,
    statuses: Mutex<BTreeMap<BundleId, super::client::ManagedCloudStatus>>,
    // Keep a superseded refresh from overwriting the next status observation.
    status_read: tokio::sync::Mutex<()>,
    /// Objects written before their manifest, per bundle.
    staged: Mutex<BTreeMap<BundleId, BTreeMap<String, Bytes>>>,
    pending: Mutex<BTreeMap<BundleId, PendingUpload>>,
}

#[derive(Clone)]
pub struct ManagedObjectStore {
    inner: Arc<Inner>,
}

impl ManagedObjectStore {
    pub fn new(client: ManagedCloudClient, account_id: String, runtime: Handle) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                runtime,
                account_id,
                repositories: Mutex::new(BTreeMap::new()),
                pinned: Mutex::new(BTreeMap::new()),
                statuses: Mutex::new(BTreeMap::new()),
                status_read: tokio::sync::Mutex::new(()),
                staged: Mutex::new(BTreeMap::new()),
                pending: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    pub fn set_pin(&self, bundle: &BundleId, id: Option<&str>) -> Result<(), String> {
        if let Some(id) = id {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err("Invalid Mallard Cloud repository ID.".to_string());
            }
            let previous = Self::lock(&self.inner.pinned)?.insert(bundle.clone(), id.to_owned());
            if previous.as_deref() != Some(id) {
                Self::lock(&self.inner.statuses)?.remove(bundle);
            }
            self.remember(bundle, id)
        } else {
            if Self::lock(&self.inner.pinned)?.remove(bundle).is_some() {
                Self::lock(&self.inner.repositories)?.remove(bundle);
            }
            Self::lock(&self.inner.statuses)?.remove(bundle);
            Ok(())
        }
    }
    pub fn status(&self, bundle: &BundleId) -> Option<super::client::ManagedCloudStatus> {
        let repository_id = Self::lock(&self.inner.pinned).ok()?.get(bundle)?.clone();
        if let Some(health) = self
            .inner
            .client
            .health()
            .filter(|health| health.suspends())
        {
            return Some(super::client::ManagedCloudStatus {
                repository_id,
                state: if health.state == crate::storage_core::ConnectionState::PermissionDenied {
                    super::client::ManagedCloudStatusState::Inaccessible
                } else {
                    super::client::ManagedCloudStatusState::SignInRequired
                },
                link: None,
                publication: None,
                message: Some(health.describe()),
            });
        }
        Self::lock(&self.inner.statuses).ok()?.get(bundle).cloned()
    }
    pub fn resolve_repository(&self, bundle: &BundleId) -> Result<String, String> {
        self.inner
            .runtime
            .block_on(self.repository_for(bundle))?
            .ok_or_else(|| {
                "Cloud repository is unavailable. Complete project setup before linking."
                    .to_string()
            })
    }
    pub fn client(&self) -> &ManagedCloudClient {
        &self.inner.client
    }

    fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
        mutex
            .lock()
            .map_err(|_| "the managed store's table is poisoned".to_string())
    }

    fn remember(&self, bundle_id: &BundleId, repository_id: &str) -> Result<(), String> {
        Self::lock(&self.inner.repositories)?.insert(bundle_id.clone(), repository_id.to_owned());
        Ok(())
    }

    /// The repository id for a bundle, from the table or the service.
    /// `None` when the account knows no repository for the bundle.
    async fn repository_for(&self, bundle_id: &BundleId) -> Result<Option<String>, String> {
        if let Some(id) = Self::lock(&self.inner.pinned)?.get(bundle_id).cloned() {
            return Ok(Some(id));
        }
        if let Some(id) = Self::lock(&self.inner.repositories)?
            .get(bundle_id)
            .cloned()
        {
            return Ok(Some(id));
        }
        let Some(answer) = self.inner.client.head_by_bundle(bundle_id.as_str()).await? else {
            return Ok(None);
        };
        self.remember(bundle_id, &answer.repository_id)?;
        Ok(Some(answer.repository_id))
    }

    async fn head_bytes(&self, bundle_id: &BundleId) -> Result<Option<StoredMetadata>, String> {
        let _status_read = self.inner.status_read.lock().await;
        let pinned = Self::lock(&self.inner.pinned)?.get(bundle_id).cloned();
        let head = if let Some(id) = pinned {
            match self.inner.client.repository_status(&id).await {
                Ok((head, status)) => {
                    Self::lock(&self.inner.statuses)?.insert(bundle_id.clone(), status);
                    head
                }
                Err(error) => {
                    let status = super::client::ManagedCloudStatus::failed(&id, &error);
                    let message = status.message.clone().unwrap_or_else(|| error.message());
                    Self::lock(&self.inner.statuses)?.insert(bundle_id.clone(), status);
                    return Err(message);
                }
            }
        } else {
            // No Cloud status checks when the optional repository ID is absent.
            let Some(answer) = self.inner.client.head_by_bundle(bundle_id.as_str()).await? else {
                return Ok(None);
            };
            self.remember(bundle_id, &answer.repository_id)?;
            answer.head
        };
        let Some(head) = head else {
            return Ok(None);
        };
        if head.bundle_id.as_deref() != Some(bundle_id.as_str()) || head.kind != "project" {
            return Err("Mallard Cloud answered with a head of another repository".to_string());
        }
        let synthesized = BundleHead {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: bundle_id.clone(),
            kind: BundleKind::Project,
            revision: head.revision,
            commit_id: head.commit_id.clone(),
            manifest_key: format!("_manifests/{}-{}.json", head.revision, head.commit_id),
            manifest_sha256: head.manifest_sha256,
            manifest_size: head.manifest_size,
            updated_at: head.updated_at,
        };
        synthesized.validate()?;
        Ok(Some(StoredMetadata {
            bytes: serde_json::to_vec(&synthesized)
                .map_err(|error| format!("encode bundle head: {error}"))?,
            etag: head.commit_id,
        }))
    }

    async fn download_into(
        &self,
        key: &Key,
        max: u64,
        sink: &mut impl Write,
    ) -> Result<Option<super::client::Downloaded>, String> {
        let (route, bundle_id, path) = match key {
            Key::Manifest {
                bundle_id,
                commit_id,
                ..
            } => (
                "v1/repositories/{id}/manifests/{commit}",
                bundle_id,
                format!("/manifests/{commit_id}"),
            ),
            Key::Object { bundle_id, sha256 } => (
                "v1/repositories/{id}/objects/{sha256}",
                bundle_id,
                format!("/objects/{sha256}"),
            ),
            _ => return Err("only manifests and objects are downloaded".to_string()),
        };
        let Some(repository_id) = self.repository_for(bundle_id).await? else {
            return Ok(None);
        };
        Ok(self
            .inner
            .client
            .download(
                route,
                &format!("/v1/repositories/{repository_id}{path}"),
                max,
                sink,
            )
            .await?)
    }

    async fn stage_verified_async(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        let classified = classify(key)?;
        match classified {
            Key::Manifest { .. } | Key::Object { .. } => {}
            Key::Library => return Ok(StageOutcome::Missing),
            _ => return Err(format!("'{key}' is not a verified object")),
        }
        mallard_domain::validate_sha256("expected object", &expected.sha256)?;
        let mut file = NamedTempFile::new().map_err(|error| format!("stage object: {error}"))?;
        let Some(downloaded) = self
            .download_into(&classified, expected.size, &mut file)
            .await?
        else {
            return Ok(StageOutcome::Missing);
        };
        if downloaded.size != expected.size || downloaded.sha256 != expected.sha256 {
            return Ok(StageOutcome::Mismatch);
        }
        Ok(StageOutcome::Verified(VerifiedBody::from_temp_file(
            file,
            downloaded.size,
            downloaded.sha256,
            downloaded.etag,
        )?))
    }

    async fn get_metadata_async(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        match classify(key)? {
            Key::Marker => Ok(Some(StoredMetadata {
                bytes: serde_json::to_vec(&StorageMarker::current())
                    .map_err(|error| format!("encode storage marker: {error}"))?,
                etag: "managed".to_string(),
            })),
            Key::Head(bundle_id) => self.head_bytes(&bundle_id).await,
            classified @ (Key::Manifest { .. } | Key::Object { .. }) => {
                let mut bytes = Vec::new();
                Ok(self
                    .download_into(&classified, max_bytes, &mut bytes)
                    .await?
                    .map(|downloaded| StoredMetadata {
                        bytes,
                        etag: downloaded.etag,
                    }))
            }
            Key::Record(_) | Key::Library => Ok(None),
            Key::Upload(_) => Err("Mallard Cloud has no probe namespace".to_string()),
        }
    }

    /// The manifest write: resolve or create the repository, end an own
    /// upload of this same manifest, build the views, create the upload, put
    /// what the service says is missing, then end the account's own stale
    /// uploads (the new upload already references the objects they carried).
    /// The commit happens at the record write.
    async fn begin_upload(
        &self,
        bundle_id: &BundleId,
        manifest_bytes: Bytes,
    ) -> Result<(), String> {
        let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("parse the manifest being pushed: {error}"))?;
        manifest.validate()?;
        if &manifest.bundle.bundle_id != bundle_id {
            return Err("the manifest being pushed names another bundle".to_string());
        }
        let client = &self.inner.client;
        let repository_id = match self.repository_for(bundle_id).await? {
            Some(id) => id,
            None => {
                let spec = CreateRepository {
                    repository_class: if manifest.bundle.repository_fingerprint.is_some() {
                        "git_project"
                    } else {
                        "non_git_project"
                    },
                    // The project title is the requested name; the service normalizes it.
                    name: manifest.bundle.display_name.clone(),
                    bundle_id: bundle_id.as_str().to_owned(),
                    repository_fingerprint: manifest.bundle.repository_fingerprint.clone(),
                };
                match client.create_repository(&spec).await {
                    Ok(created) => {
                        self.remember(bundle_id, &created.id)?;
                        created.id
                    }
                    // Lost a race with another replica of this account.
                    Err(error) if error.kind == ErrorKind::AccountExists => self
                        .repository_for(bundle_id)
                        .await?
                        .ok_or_else(|| error.message())?,
                    Err(error) => return Err(error.message()),
                }
            }
        };
        let summary = client.get_repository(&repository_id).await?;
        // Uploads run side by side on the service and the commit is the only
        // compare-and-swap, so another account's open upload is left alone.
        // An own upload of this very manifest is ended first, because the
        // service claims a commit id per open upload; own stale uploads are
        // ended only after the new upload exists, so the objects they carried
        // stay protected and are reused instead of re-sent.
        let own: Vec<_> = summary
            .open_uploads
            .iter()
            .filter(|open| open.user_id == self.inner.account_id)
            .cloned()
            .collect();
        for open in own
            .iter()
            .filter(|open| open.commit_id == manifest.commit_id)
        {
            client.abandon_upload(&repository_id, &open.id).await?;
        }
        let stale: Vec<_> = own
            .iter()
            .filter(|open| open.stale && open.commit_id != manifest.commit_id)
            .collect();
        let staged = Self::lock(&self.inner.staged)?
            .get(bundle_id)
            .cloned()
            .unwrap_or_default();
        let views =
            build_views_for_manifest(AgentProviderRegistry::production(), &manifest, &staged)?;
        let mut objects = BTreeMap::new();
        for (_, entry) in manifest.all_files() {
            objects.insert(
                entry.sha256.clone(),
                InventoryEntry {
                    sha256: entry.sha256.clone(),
                    size: entry.size,
                    role: "payload",
                    resource_id: None,
                    source_sha256: None,
                },
            );
        }
        let mut view_bytes = BTreeMap::new();
        for view in &views {
            objects.insert(
                view.sha256.clone(),
                InventoryEntry {
                    sha256: view.sha256.clone(),
                    size: view.bytes.len() as u64,
                    role: "view",
                    resource_id: Some(view.resource_id.clone()),
                    source_sha256: Some(view.source_sha256.clone()),
                },
            );
            view_bytes.insert(view.sha256.clone(), view.bytes.clone());
        }
        let request = UploadRequest {
            expected_repository_revision: summary.revision,
            expected_private_revision: summary.private_revision,
            objects: objects.into_values().collect(),
        };
        let mut stale_ended = false;
        let created = match client
            .create_upload(&repository_id, &request, manifest_bytes.clone())
            .await
        {
            Ok(created) => created,
            // Past the per-member cap: make room with the stale ones and retry once.
            Err(error) if error.kind == ErrorKind::UploadInProgress && !stale.is_empty() => {
                for open in &stale {
                    client.abandon_upload(&repository_id, &open.id).await?;
                }
                stale_ended = true;
                client
                    .create_upload(&repository_id, &request, manifest_bytes)
                    .await?
            }
            Err(error) => return Err(error.message()),
        };
        if !stale_ended {
            for open in &stale {
                let _ = client.abandon_upload(&repository_id, &open.id).await;
            }
        }
        let mut transfers = Vec::with_capacity(created.missing.len());
        for missing in &created.missing {
            let bytes = staged
                .get(&missing.sha256)
                .or_else(|| view_bytes.get(&missing.sha256))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "Mallard Cloud needs object {} which this push did not capture; compare again before pushing",
                        missing.sha256
                    )
                })?;
            transfers.push((missing.sha256.clone(), bytes));
        }
        let concurrency = crate::app_settings::cloud_transfer_concurrency().max(1);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut workers = tokio::task::JoinSet::new();
        for (sha256, bytes) in transfers {
            let client = client.clone();
            let repository_id = repository_id.clone();
            let upload_id = created.upload_id.clone();
            let semaphore = semaphore.clone();
            workers.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| Error::local("v1/repositories/{id}/uploads/{upload}/objects"))?;
                client
                    .put_object(&repository_id, &upload_id, &sha256, bytes)
                    .await
            });
        }
        while let Some(joined) = workers.join_next().await {
            let outcome = joined.unwrap_or_else(|_| {
                Err(Error::local(
                    "v1/repositories/{id}/uploads/{upload}/objects",
                ))
            });
            if let Err(error) = outcome {
                workers.abort_all();
                while workers.join_next().await.is_some() {}
                let _ = client
                    .abandon_upload(&repository_id, &created.upload_id)
                    .await;
                return Err(error.message());
            }
        }
        Self::lock(&self.inner.pending)?.insert(
            bundle_id.clone(),
            PendingUpload {
                repository_id,
                upload_id: created.upload_id,
                commit_id: created.commit_id,
                revision: manifest.revision,
            },
        );
        Ok(())
    }

    /// The record write commits the upload. The service keeps its commit
    /// guard: it refuses a record whose parent is not its current head, which
    /// is a changed remote with nothing written.
    async fn commit_record(
        &self,
        bundle_id: &BundleId,
        bytes: &[u8],
    ) -> Result<ImmutablePutOutcome, String> {
        #[derive(serde::Deserialize)]
        struct RecordFacts {
            revision: u64,
            commit_id: String,
            #[serde(default)]
            previous_commit_id: Option<String>,
        }
        let record: RecordFacts = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse the record being written: {error}"))?;
        let pending = Self::lock(&self.inner.pending)?.remove(bundle_id);
        Self::lock(&self.inner.staged)?.remove(bundle_id);
        let Some(pending) = pending else {
            return Err("a record cannot be written before its manifest was uploaded".to_string());
        };
        if pending.commit_id != record.commit_id || pending.revision != record.revision {
            let _ = self
                .inner
                .client
                .abandon_upload(&pending.repository_id, &pending.upload_id)
                .await;
            return Err("the record being written does not name the uploaded manifest".to_string());
        }
        match self
            .inner
            .client
            .commit_upload(
                &pending.repository_id,
                &pending.upload_id,
                record.previous_commit_id.as_deref(),
            )
            .await
        {
            Ok(_) => Ok(ImmutablePutOutcome::Written),
            // The upload is stale now. It stays open so the next push, built
            // on the moved head, reuses the objects it carried; that push ends
            // it, or it expires.
            Err(error) if error.kind == ErrorKind::HeadChanged => Err(REMOTE_CHANGED.to_string()),
            Err(error) => Err(error.message()),
        }
    }

    /// Nothing is enumerable: Library reads are absent, and the service
    /// lists no publication records yet.
    fn enumerate(prefix: &ObjectPrefix, limit: usize) -> Result<StoreListPage, String> {
        if limit == 0 {
            return Err("listing limit must be positive".to_string());
        }
        let raw = prefix.as_str();
        if raw == LIBRARY_PREFIX || raw.starts_with(&format!("{LIBRARY_PREFIX}/")) {
            return Ok(StoreListPage {
                entries: vec![],
                next_cursor: None,
            });
        }
        Err("Mallard Cloud does not list publication records yet".to_string())
    }
}

impl ObjectStore for ManagedObjectStore {
    fn managed_cloud(&self) -> Option<&ManagedObjectStore> {
        Some(self)
    }

    fn connection_health(&self) -> Option<ConnectionHealth> {
        self.inner.client.health()
    }

    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.inner
            .runtime
            .block_on(self.get_metadata_async(key, max_bytes))
    }

    fn get_metadata_many(
        &self,
        keys: &[ObjectKey],
        max_bytes: u64,
    ) -> Result<Vec<Option<StoredMetadata>>, String> {
        // Tag reads share one listing; heads and objects are few per call.
        self.inner.runtime.block_on(async {
            let mut out = Vec::with_capacity(keys.len());
            for key in keys {
                out.push(self.get_metadata_async(key, max_bytes).await?);
            }
            Ok(out)
        })
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.inner
            .runtime
            .block_on(self.stage_verified_async(key, expected))
    }

    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> Result<Vec<StageOutcome>, String> {
        if requests.len() <= 1 {
            return requests
                .iter()
                .map(|(key, expected)| self.stage_verified(key, expected))
                .collect();
        }
        let concurrency = crate::app_settings::cloud_transfer_concurrency().max(1);
        self.inner.runtime.block_on(async {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
            let mut workers = tokio::task::JoinSet::new();
            for (index, (key, expected)) in requests.iter().cloned().enumerate() {
                let store = self.clone();
                let semaphore = semaphore.clone();
                workers.spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .map_err(|_| "the transfer limiter closed".to_string())?;
                    store
                        .stage_verified_async(&key, &expected)
                        .await
                        .map(|outcome| (index, outcome))
                });
            }
            let mut slots: Vec<Option<StageOutcome>> = Vec::with_capacity(requests.len());
            slots.resize_with(requests.len(), || None);
            while let Some(joined) = workers.join_next().await {
                match joined.unwrap_or_else(|_| Err("a transfer task failed".to_string())) {
                    Ok((index, outcome)) => slots[index] = Some(outcome),
                    Err(error) => {
                        workers.abort_all();
                        while workers.join_next().await.is_some() {}
                        return Err(error);
                    }
                }
            }
            slots
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| "a transfer produced no outcome".to_string())
        })
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        self.put_many(vec![(key.clone(), Bytes::copy_from_slice(bytes))])?
            .pop()
            .ok_or_else(|| "no put outcome".to_string())
    }

    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        let mut outcomes = Vec::with_capacity(objects.len());
        for (key, bytes) in objects {
            match classify(&key)? {
                Key::Object { bundle_id, sha256 } => {
                    if sha256_hex(&bytes) != sha256 {
                        return Err(format!("object '{key}' does not match its digest"));
                    }
                    Self::lock(&self.inner.staged)?
                        .entry(bundle_id)
                        .or_default()
                        .insert(sha256, bytes);
                    outcomes.push(ImmutablePutOutcome::Written);
                }
                Key::Manifest { bundle_id, .. } => {
                    self.inner
                        .runtime
                        .block_on(self.begin_upload(&bundle_id, bytes))?;
                    outcomes.push(ImmutablePutOutcome::Written);
                }
                Key::Marker => outcomes.push(ImmutablePutOutcome::AlreadyPresent),
                Key::Library => return Err(CATALOG_REFUSAL.to_string()),
                Key::Upload(_) => return Err("Mallard Cloud has no probe namespace".to_string()),
                Key::Record(_) => {
                    return Err(format!("'{key}' is written as a publication record only"))
                }
                Key::Head(_) => return Err(format!("'{key}' is never written")),
            }
        }
        Ok(outcomes)
    }

    fn put_record(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        match classify(key)? {
            Key::Record(bundle_id) => self
                .inner
                .runtime
                .block_on(self.commit_record(&bundle_id, bytes)),
            Key::Library => Err(CATALOG_REFUSAL.to_string()),
            _ => Err(format!("'{key}' is not a publication record")),
        }
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        _cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        Self::enumerate(prefix, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(number: u64) -> BundleId {
        BundleId::parse(format!("{number:032x}")).unwrap()
    }

    #[test]
    fn keys_classify_into_the_behaviour_table() {
        let id = bundle(7);
        let object =
            |relative: &str| crate::storage_core::repository_object_key(&id, relative).unwrap();
        assert_eq!(
            classify(&ObjectKey::parse(STORAGE_MARKER_KEY).unwrap()).unwrap(),
            Key::Marker
        );
        let head = ObjectKey::parse(format!("{REPOSITORY_PREFIX}/{id}/_head.json")).unwrap();
        assert_eq!(classify(&head).unwrap(), Key::Head(id.clone()));
        assert_eq!(
            classify(&object("_manifests/3-commit-abcdef12.json")).unwrap(),
            Key::Manifest {
                bundle_id: id.clone(),
                revision: 3,
                commit_id: "commit-abcdef12".into()
            }
        );
        let record =
            ObjectKey::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{id}/record.json")).unwrap();
        assert_eq!(classify(&record).unwrap(), Key::Record(id.clone()));
        let sha = "a".repeat(64);
        assert_eq!(
            classify(&object(&format!("_objects/{sha}"))).unwrap(),
            Key::Object {
                bundle_id: id.clone(),
                sha256: sha
            }
        );
        assert_eq!(
            classify(&object("_uploads/probe")).unwrap(),
            Key::Upload(id)
        );
        assert_eq!(
            classify(
                &crate::storage_core::library_object_key(
                    "library-0123456789abcdef0123456789abcdef",
                    "_tag.json"
                )
                .unwrap()
            )
            .unwrap(),
            Key::Library
        );
    }
}

#[cfg(test)]
mod stub_tests {
    use super::*;
    use crate::managed_cloud::stub::StubManagedCloud;
    use crate::storage_core::publication::RecordStream;
    use crate::storage_core::repository_object_key;
    use mallard_domain::{
        BundleIdentity, OriginHostToken, OriginReplicaToken, PortableOrigin, PublicationToken,
        RecordName, SecretValue,
    };

    fn origin() -> PortableOrigin {
        PortableOrigin {
            origin_host_token: OriginHostToken::parse("a".repeat(64)).unwrap(),
            origin_replica_token: OriginReplicaToken::parse("b".repeat(64)).unwrap(),
            shared_label: None,
        }
    }

    fn identity(number: u64) -> BundleIdentity {
        BundleIdentity {
            bundle_id: bundle(number),
            display_name: "Cloud project".into(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        }
    }

    fn bundle(number: u64) -> BundleId {
        BundleId::parse(format!("{number:032x}")).unwrap()
    }

    async fn store(stub: &StubManagedCloud, login: &str) -> (ManagedObjectStore, String) {
        let (user, token) = stub.state.lock().unwrap().add_user(login);
        let client =
            ManagedCloudClient::new(&stub.endpoint, Some(SecretValue::parse(&token).unwrap()))
                .unwrap();
        (
            ManagedObjectStore::new(client, user.id.clone(), Handle::current()),
            user.id,
        )
    }

    fn manifest_key(manifest: &BundleManifest) -> ObjectKey {
        repository_object_key(
            &manifest.bundle.bundle_id,
            &format!(
                "_manifests/{}-{}.json",
                manifest.revision, manifest.commit_id
            ),
        )
        .unwrap()
    }

    /// The publication record of `manifest` on `parent`, and its key. The
    /// service reads the commit facts from it and nothing else.
    fn record_of(manifest: &BundleManifest, parent: Option<&str>) -> (ObjectKey, Vec<u8>) {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "bundle_id": manifest.bundle.bundle_id,
            "revision": manifest.revision,
            "commit_id": manifest.commit_id,
            "previous_commit_id": parent,
        }))
        .unwrap();
        let name = RecordName::new(
            manifest.revision,
            PublicationToken::new(manifest.commit_id.clone(), sha256_hex(&bytes)).unwrap(),
            parent.map(|parent| PublicationToken::new(parent, "0".repeat(64)).unwrap()),
            manifest.updated_at,
        )
        .unwrap();
        let key = RecordStream::project(&manifest.bundle.bundle_id)
            .unwrap()
            .key(&name)
            .unwrap();
        (key, bytes)
    }

    /// Pushes a manifest on top of `parent`: the manifest write creates the
    /// upload, the record write commits it.
    async fn push(
        store: &ManagedObjectStore,
        manifest: &BundleManifest,
        parent: Option<&str>,
    ) -> Result<ImmutablePutOutcome, String> {
        let manifest_key = manifest_key(manifest);
        let manifest_bytes = Bytes::from(serde_json::to_vec(manifest).unwrap());
        let (record_key, record_bytes) = record_of(manifest, parent);
        let store = store.clone();
        tokio::task::spawn_blocking(move || {
            store
                .put_many(vec![(manifest_key, manifest_bytes)])
                .unwrap();
            store.put_record(&record_key, &record_bytes)
        })
        .await
        .unwrap()
    }

    /// The empty registration manifest of a just-linked bundle, pushed as
    /// the repository's first publication.
    async fn register(store: &ManagedObjectStore, identity: BundleIdentity) -> BundleManifest {
        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            revision: 1,
            commit_id: mallard_domain::generated_named_id("commit").unwrap(),
            updated_at: 100,
            bundle: identity,
            published_by_origin: origin(),
            scopes: BTreeMap::new(),
        };
        push(store, &manifest, None).await.unwrap();
        manifest
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinned_status_shares_one_details_read_and_never_recreates_a_missing_repository() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "status-owner").await;
        register(&store, identity(91)).await;
        let id = stub
            .state
            .lock()
            .unwrap()
            .repository_for_bundle(bundle(91).as_str())
            .unwrap()
            .id
            .clone();
        store.set_pin(&bundle(91), Some(&id)).unwrap();
        stub.state.lock().unwrap().log.clear();
        assert!(store.head_bytes(&bundle(91)).await.unwrap().is_some());
        let requests = stub.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, format!("/v1/repositories/{id}"));
        assert_eq!(requests[1].path, format!("/v1/repositories/{id}/head"));
        let observed = store.status(&bundle(91)).unwrap();
        assert_eq!(
            observed.state,
            super::super::client::ManagedCloudStatusState::Ready
        );
        assert_eq!(
            observed.link.unwrap().release,
            super::super::client::ReleaseState::NotPushed
        );
        stub.state.lock().unwrap().log.clear();
        let bundle_id = bundle(91);
        let (first, second) =
            tokio::join!(store.head_bytes(&bundle_id), store.head_bytes(&bundle_id));
        assert!(first.unwrap().is_some());
        assert!(second.unwrap().is_some());
        assert_eq!(
            stub.requests()
                .iter()
                .map(|request| request.path.clone())
                .collect::<Vec<_>>(),
            vec![
                format!("/v1/repositories/{id}"),
                format!("/v1/repositories/{id}/head"),
                format!("/v1/repositories/{id}"),
                format!("/v1/repositories/{id}/head"),
            ]
        );
        stub.state.lock().unwrap().fail_next_summary = true;
        assert!(store.head_bytes(&bundle(91)).await.unwrap().is_some());
        let partial = store.status(&bundle(91)).unwrap();
        assert_eq!(
            partial.state,
            super::super::client::ManagedCloudStatusState::Ready
        );
        assert_eq!(
            partial.publication.unwrap().state,
            super::super::client::PublicationState::Unavailable
        );
        stub.state.lock().unwrap().repositories.remove(&id);
        assert!(store
            .head_bytes(&bundle(91))
            .await
            .unwrap_err()
            .contains("deleted or you no longer have access"));
        assert_eq!(
            store.status(&bundle(91)).unwrap().state,
            super::super::client::ManagedCloudStatusState::Inaccessible
        );
        assert_eq!(
            store.repository_for(&bundle(91)).await.unwrap().as_deref(),
            Some(id.as_str())
        );
        assert!(!stub
            .requests()
            .iter()
            .any(|request| request.method == "POST"));
        // Removing optional metadata disables the new status path, even with a warm store.
        store.set_pin(&bundle(91), None).unwrap();
        assert!(store.status(&bundle(91)).is_none());
        stub.state.lock().unwrap().log.clear();
        assert!(store.head_bytes(&bundle(91)).await.unwrap().is_none());
        assert!(stub.requests()[0].path.contains("/by-bundle/"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinned_status_detects_privacy_access_loss_and_recovers() {
        let stub = StubManagedCloud::start().await;
        let (owner, _) = store(&stub, "private-owner").await;
        let (contributor, _) = store(&stub, "contributor").await;
        register(&owner, identity(92)).await;
        let id = stub
            .state
            .lock()
            .unwrap()
            .repository_for_bundle(bundle(92).as_str())
            .unwrap()
            .id
            .clone();
        stub.state.lock().unwrap().release(&id, None);
        owner.set_pin(&bundle(92), Some(&id)).unwrap();
        contributor.set_pin(&bundle(92), Some(&id)).unwrap();
        contributor.head_bytes(&bundle(92)).await.unwrap();
        stub.state
            .lock()
            .unwrap()
            .repositories
            .get_mut(&id)
            .unwrap()
            .visibility = "private".into();
        assert!(contributor.head_bytes(&bundle(92)).await.is_err());
        owner.head_bytes(&bundle(92)).await.unwrap();
        assert_eq!(
            owner.status(&bundle(92)).unwrap().link.unwrap().visibility,
            "private"
        );
        stub.state.lock().unwrap().release(&id, None);
        contributor.head_bytes(&bundle(92)).await.unwrap();
        assert_eq!(
            contributor.status(&bundle(92)).unwrap().state,
            super::super::client::ManagedCloudStatusState::Ready
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_first_publication_creates_the_repository_and_commits_on_the_record_write() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        register(&store, identity(1)).await;
        let state = stub.state.lock().unwrap();
        assert_eq!(state.repositories.len(), 1);
        let repository = state.repository_for_bundle(bundle(1).as_str()).unwrap();
        assert_eq!(repository.class, "non_git_project");
        assert_eq!(
            repository.private_head.as_ref().unwrap().manifest_revision,
            1
        );
        // Every create and commit carried an Idempotency-Key.
        for request in state.log.iter().filter(|r| {
            r.method == "POST"
                && (r.path == "/v1/repositories"
                    || r.path.ends_with("/commit")
                    || r.path.ends_with("/uploads"))
        }) {
            assert!(request.idempotency_key.is_some(), "{request:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_keeps_the_requested_github_name_and_suffixes_conflicts() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        for number in [1, 2] {
            let mut requested = identity(number);
            requested.display_name = "My--Repo".into();
            requested.repository_fingerprint = Some("a".repeat(64));
            register(&store, requested).await;
        }
        let state = stub.state.lock().unwrap();
        assert_eq!(
            state
                .repository_for_bundle(bundle(1).as_str())
                .unwrap()
                .name,
            "My--Repo"
        );
        assert_eq!(
            state
                .repository_for_bundle(bundle(2).as_str())
                .unwrap()
                .name,
            "My--Repo-2"
        );
    }

    fn next_manifest(previous: &BundleManifest) -> BundleManifest {
        let mut manifest = previous.clone();
        manifest.revision += 1;
        manifest.commit_id = mallard_domain::generated_named_id("commit").unwrap();
        manifest.updated_at += 1;
        manifest
    }

    /// Creates the upload for a manifest without committing it; returns its id.
    async fn open_upload(
        stub: &StubManagedCloud,
        store: &ManagedObjectStore,
        manifest: &BundleManifest,
    ) -> String {
        let key = manifest_key(manifest);
        let bytes = Bytes::from(serde_json::to_vec(manifest).unwrap());
        let store = store.clone();
        tokio::task::spawn_blocking(move || store.put_many(vec![(key, bytes)]).unwrap())
            .await
            .unwrap();
        let state = stub.state.lock().unwrap();
        state
            .uploads
            .values()
            .find(|u| u.commit_id == manifest.commit_id && u.state == "open")
            .unwrap()
            .id
            .clone()
    }

    fn current_manifest(stub: &StubManagedCloud, number: u64) -> BundleManifest {
        let state = stub.state.lock().unwrap();
        let head = state
            .repository_for_bundle(bundle(number).as_str())
            .unwrap()
            .private_head
            .as_ref()
            .unwrap();
        serde_json::from_slice(&head.manifest).unwrap()
    }

    /// Positions in the stub's request log of the last upload creation and of
    /// the abandon of `upload_id`: the replacement must exist before a stale
    /// upload is ended, so the objects it carried stay protected.
    fn created_before_ended(stub: &StubManagedCloud, upload_id: &str) -> bool {
        let state = stub.state.lock().unwrap();
        let created = state
            .log
            .iter()
            .rposition(|r| r.method == "POST" && r.path.ends_with("/uploads"))
            .unwrap();
        let ended = state
            .log
            .iter()
            .rposition(|r| {
                r.method == "POST" && r.path.ends_with(&format!("/uploads/{upload_id}/abandon"))
            })
            .unwrap();
        created < ended
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn own_uploads_are_ended_in_order_and_a_collaborators_push_coexists() {
        let stub = StubManagedCloud::start().await;
        let (store, me) = store(&stub, "octocat").await;
        let base = register(&store, identity(2)).await;
        let id = bundle(2);
        let revision_of = |stub: &StubManagedCloud| {
            stub.state
                .lock()
                .unwrap()
                .repository_for_bundle(id.as_str())
                .unwrap()
                .private_head
                .as_ref()
                .unwrap()
                .manifest_revision
        };

        // Two own leftovers, never committed: one of the very manifest pushed next, one of another.
        let second = next_manifest(&base);
        let same = open_upload(&stub, &store, &second).await;
        let other = open_upload(&stub, &store, &next_manifest(&base)).await;
        assert_eq!(stub.state.lock().unwrap().uploads[&same].user_id, me);
        // A new process knows neither.
        store.inner.pending.lock().unwrap().clear();
        push(&store, &second, Some(&base.commit_id)).await.unwrap();
        {
            let state = stub.state.lock().unwrap();
            // The same-manifest leftover had to go first; the other is not stale yet and is left alone.
            assert_eq!(state.uploads[&same].state, "abandoned");
            assert_eq!(state.uploads[&other].state, "open");
        }
        assert_eq!(revision_of(&stub), 2);

        // The head moved, so the other leftover is stale: the next push ends it, after its own upload exists.
        let third = next_manifest(&current_manifest(&stub, 2));
        push(&store, &third, Some(&second.commit_id)).await.unwrap();
        assert_eq!(
            stub.state.lock().unwrap().uploads[&other].state,
            "abandoned"
        );
        assert!(created_before_ended(&stub, &other));
        assert_eq!(revision_of(&stub), 3);

        // A collaborator's open upload never blocks a push and is never touched.
        let (rika_store, rika) = store_for_member(&stub, "rika321", id.as_str()).await;
        let shared_base = current_manifest(&stub, 2);
        let theirs_manifest = next_manifest(&shared_base);
        let theirs = open_upload(&stub, &rika_store, &theirs_manifest).await;
        let fourth = next_manifest(&shared_base);
        push(&store, &fourth, Some(&third.commit_id)).await.unwrap();
        {
            let state = stub.state.lock().unwrap();
            assert_eq!(state.uploads[&theirs].state, "open");
            assert_eq!(state.uploads[&theirs].user_id, rika);
        }

        // Their record names a parent that is no longer the head: a changed
        // remote that leaves the upload open...
        let conflict = tokio::task::spawn_blocking({
            let store = rika_store.clone();
            let (record_key, record_bytes) = record_of(&theirs_manifest, Some(&third.commit_id));
            move || store.put_record(&record_key, &record_bytes)
        })
        .await
        .unwrap();
        assert_eq!(conflict.unwrap_err(), REMOTE_CHANGED);
        assert_eq!(stub.state.lock().unwrap().uploads[&theirs].state, "open");
        // ...and their push on the moved head ends it only once the replacement exists.
        let fifth = next_manifest(&current_manifest(&stub, 2));
        push(&rika_store, &fifth, Some(&fourth.commit_id))
            .await
            .unwrap();
        assert_eq!(
            stub.state.lock().unwrap().uploads[&theirs].state,
            "abandoned"
        );
        assert!(created_before_ended(&stub, &theirs));
        assert_eq!(revision_of(&stub), 5);
    }

    async fn store_for_member(
        stub: &StubManagedCloud,
        login: &str,
        bundle_id: &str,
    ) -> (ManagedObjectStore, String) {
        let (store, user_id) = store(stub, login).await;
        let repository_id = stub
            .state
            .lock()
            .unwrap()
            .repository_for_bundle(bundle_id)
            .unwrap()
            .id
            .clone();
        stub.state
            .lock()
            .unwrap()
            .add_member(&repository_id, &user_id, "member");
        (store, user_id)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_moved_head_is_a_changed_remote_that_leaves_the_upload_open() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let registered = register(&store, identity(3)).await;
        stub.state.lock().unwrap().fail_next_commit = true;
        let outcome = push(
            &store,
            &next_manifest(&registered),
            Some(&registered.commit_id),
        )
        .await;
        assert_eq!(outcome.unwrap_err(), REMOTE_CHANGED);
        let state = stub.state.lock().unwrap();
        // The upload stays open: the next push, built on the moved head, reuses what it carried.
        assert_eq!(
            state.uploads.values().filter(|u| u.state == "open").count(),
            1
        );
        assert!(store.inner.pending.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_revoked_session_suspends_the_store_and_library_keys_are_inert() {
        let stub = StubManagedCloud::start().await;
        let (user, token) = stub.state.lock().unwrap().add_user("octocat");
        let client =
            ManagedCloudClient::new(&stub.endpoint, Some(SecretValue::parse(&token).unwrap()))
                .unwrap();
        let store = ManagedObjectStore::new(client, user.id, Handle::current());
        let marker = ObjectKey::parse(STORAGE_MARKER_KEY).unwrap();
        let library = crate::storage_core::library_object_key(
            "library-0123456789abcdef0123456789abcdef",
            "_tag.json",
        )
        .unwrap();
        stub.state.lock().unwrap().revoke_token(&token);
        let (marker_read, head_read, health, library_read, library_write) =
            tokio::task::spawn_blocking(move || {
                let marker_read = store.get_metadata(&marker, 4096).unwrap();
                let head_key =
                    ObjectKey::parse(format!("{REPOSITORY_PREFIX}/{}/_head.json", bundle(4)))
                        .unwrap();
                let head_read = store.get_metadata(&head_key, 4096);
                let health = store.connection_health().unwrap();
                let library_read = store.get_metadata(&library, 4096).unwrap();
                let library_write = store.put_immutable(&library, b"{}").unwrap_err();
                (marker_read, head_read, health, library_read, library_write)
            })
            .await
            .unwrap();
        assert!(marker_read.is_some(), "the marker needs no request");
        assert!(head_read.unwrap_err().contains("Reconnect"));
        assert_eq!(
            health.state,
            crate::storage_core::ConnectionState::ReconnectRequired
        );
        assert!(health.suspends());
        assert!(library_read.is_none());
        assert!(library_write.starts_with("managed_catalog_uses_item_adapter"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn desktop_sign_in_round_trips_through_the_loopback_receiver() {
        let stub = StubManagedCloud::start().await;
        stub.state.lock().unwrap().add_user("octocat");
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handle = Handle::current();
        let signed = crate::managed_cloud::sign_in(
            &stub.endpoint,
            move |url| {
                // The "browser": the consent page redirects to the loopback.
                let consent = format!("{}&login=octocat", url.as_str());
                handle.spawn(async move {
                    let _ = reqwest::get(consent).await;
                });
                Ok(())
            },
            cancelled,
        )
        .await
        .map_err(|error| error.code())
        .unwrap();
        assert_eq!(signed.user.github_login, "octocat");
        let client = ManagedCloudClient::new(&stub.endpoint, Some(signed.token)).unwrap();
        assert_eq!(client.me().await.unwrap().user.github_login, "octocat");
    }
}
