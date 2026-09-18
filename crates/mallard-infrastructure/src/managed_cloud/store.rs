//! The managed store: the bundle engine's object-store contract answered by
//! the Mallard Cloud service. Heads, tags, and the storage marker are
//! synthesized from the service's answers; objects are staged in memory
//! until the manifest arrives, which turns into the three-step upload; the
//! head compare-and-swap is the commit.
//!
//! | Engine key | Behaviour |
//! | --- | --- |
//! | `.mallard/_storage.json` | synthesized marker |
//! | `_head.json` read | exact repository head when pinned, otherwise by bundle; etag is the commit id |
//! | `_manifests/{rev}-{commit}.json` read | streamed manifest, digest checked |
//! | `_objects/{sha}` read | streamed object, digest checked |
//! | `_objects/{sha}` write | staged in memory |
//! | `_manifests/…` + `_commits/…` write | create the upload, put the missing objects |
//! | `_head.json` CAS | commit the upload |
//! | `_tags/…` | read from one repository listing; writes are no-ops |
//! | `_uploads/…` | refused: probes are for user-owned stores |
//! | `.mallard/v1/library/…` | reads absent; writes refused |
use super::client::{
    CreateRepository, Error, ErrorKind, InventoryEntry, ManagedCloudClient, RepositorySummary,
    UploadRequest,
};
use super::views::build_views_for_manifest;
use crate::bundle_engine::BundleTag;
use crate::providers::AgentProviderRegistry;
use crate::storage_core::{
    CasExpectation, CasOutcome, ConnectionHealth, ExpectedObject, ImmutablePutOutcome, ObjectKey,
    ObjectPrefix, ObjectStore, StageOutcome, StorageMarker, StoreListEntry, StoreListPage,
    StoredMetadata, VerifiedBody, LIBRARY_PREFIX, REPOSITORY_PREFIX, REPOSITORY_TAGS_PREFIX,
    STORAGE_MARKER_KEY,
};
use bytes::Bytes;
use mallard_domain::{
    sha256_hex, BundleHead, BundleId, BundleKind, BundleManifest, ProviderKey, BUNDLE_SCHEMA,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use tokio::runtime::Handle;

const LISTING_FRESH_FOR: Duration = Duration::from_secs(60);

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
    Commit(BundleId),
    Object {
        bundle_id: BundleId,
        sha256: String,
    },
    Upload(BundleId),
    Tag(BundleId),
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
    if let Some(rest) = raw.strip_prefix(&format!("{REPOSITORY_TAGS_PREFIX}/")) {
        let bundle = rest
            .strip_suffix(".json")
            .ok_or_else(|| format!("unexpected tag key '{raw}'"))?;
        return Ok(Key::Tag(BundleId::parse(bundle.to_string())?));
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
    if relative.starts_with("_commits/") {
        return Ok(Key::Commit(bundle_id));
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

/// The upload created for a bundle's next snapshot, waiting for the head
/// swing that commits it.
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
    /// Objects written before their manifest, per bundle.
    staged: Mutex<BTreeMap<BundleId, BTreeMap<String, Bytes>>>,
    pending: Mutex<BTreeMap<BundleId, PendingUpload>>,
    listing: Mutex<Option<(Instant, Vec<RepositorySummary>)>>,
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
                staged: Mutex::new(BTreeMap::new()),
                pending: Mutex::new(BTreeMap::new()),
                listing: Mutex::new(None),
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

    /// One `GET /v1/repositories?class=project`, reused for a minute so a
    /// listing followed by its tag reads costs one request.
    async fn listing(&self, fresh: bool) -> Result<Vec<RepositorySummary>, String> {
        if !fresh {
            if let Some((taken, rows)) = Self::lock(&self.inner.listing)?.as_ref() {
                if taken.elapsed() < LISTING_FRESH_FOR {
                    return Ok(rows.clone());
                }
            }
        }
        let rows = self.inner.client.list_project_repositories().await?;
        for row in &rows {
            if let Some(bundle) = row.bundle_id.as_deref() {
                if let Ok(bundle_id) = BundleId::parse(bundle.to_string()) {
                    self.remember(&bundle_id, &row.id)?;
                }
            }
        }
        *Self::lock(&self.inner.listing)? = Some((Instant::now(), rows.clone()));
        Ok(rows)
    }

    fn tag_bytes(row: &RepositorySummary) -> Result<Option<(Vec<u8>, String)>, String> {
        let Some(bundle) = row.bundle_id.as_deref() else {
            return Ok(None);
        };
        let Some(summary) = row.private_summary.as_ref().or(row.public_summary.as_ref()) else {
            return Ok(None);
        };
        let head = row.private_head.as_ref().or(row.public_head.as_ref());
        let tag = BundleTag {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: BundleId::parse(bundle.to_string())?,
            display_name: summary.display_name.clone(),
            kind: BundleKind::Project,
            revision: summary.revision,
            updated_at: summary.updated_at,
            resources: summary.resources,
            files: summary.files,
            registration_only: summary.registration_only,
            providers: summary
                .providers
                .iter()
                .filter_map(|provider| ProviderKey::parse(provider).ok())
                .collect(),
            repository_fingerprint: row.repository_fingerprint.clone(),
        };
        let bytes =
            serde_json::to_vec(&tag).map_err(|error| format!("encode bundle tag: {error}"))?;
        let etag = head
            .map(|head| head.commit_id.clone())
            .unwrap_or_else(|| format!("revision-{}", summary.revision));
        Ok(Some((bytes, etag)))
    }

    async fn head_bytes(&self, bundle_id: &BundleId) -> Result<Option<StoredMetadata>, String> {
        let pinned = Self::lock(&self.inner.pinned)?.get(bundle_id).cloned();
        let head = if let Some(id) = pinned {
            match self.inner.client.repository_head(&id).await {
                Ok(answer) => {
                    Self::lock(&self.inner.statuses)?.insert(
                        bundle_id.clone(),
                        super::client::ManagedCloudStatus {
                            repository_id: id.clone(),
                            state: super::client::ManagedCloudStatusState::Ready,
                            link: Some(answer.link),
                            message: None,
                        },
                    );
                    answer.head
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
            Key::Tag(bundle_id) => {
                let rows = self.listing(false).await?;
                let row = rows
                    .iter()
                    .find(|row| row.bundle_id.as_deref() == Some(bundle_id.as_str()));
                Ok(row
                    .map(Self::tag_bytes)
                    .transpose()?
                    .flatten()
                    .map(|(bytes, etag)| StoredMetadata { bytes, etag }))
            }
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
            Key::Commit(_) | Key::Library => Ok(None),
            Key::Upload(_) => Err("Mallard Cloud has no probe namespace".to_string()),
        }
    }

    /// The manifest write: resolve or create the repository, end an own
    /// upload of this same manifest, build the views, create the upload, put
    /// what the service says is missing, then end the account's own stale
    /// uploads (the new upload already references the objects they carried).
    /// The commit happens at the head compare-and-swap.
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

    async fn commit_head(
        &self,
        bundle_id: &BundleId,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        let head: BundleHead = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse the head being written: {error}"))?;
        head.validate()?;
        let pending = Self::lock(&self.inner.pending)?.remove(bundle_id);
        Self::lock(&self.inner.staged)?.remove(bundle_id);
        let Some(pending) = pending else {
            return Err("the bundle head cannot move before its manifest was uploaded".to_string());
        };
        if pending.commit_id != head.commit_id || pending.revision != head.revision {
            let _ = self
                .inner
                .client
                .abandon_upload(&pending.repository_id, &pending.upload_id)
                .await;
            return Err("the head being written does not name the uploaded manifest".to_string());
        }
        let expected = match expectation {
            CasExpectation::Absent => None,
            CasExpectation::Match(commit_id) => Some(commit_id.as_str()),
        };
        match self
            .inner
            .client
            .commit_upload(&pending.repository_id, &pending.upload_id, expected)
            .await
        {
            Ok(committed) => {
                *Self::lock(&self.inner.listing)? = None;
                Ok(CasOutcome::Written {
                    etag: committed.head.commit_id,
                })
            }
            // The upload is stale now. It stays open so the next push, built
            // on the moved head, reuses the objects it carried; that push ends
            // it, or it expires.
            Err(error) if error.kind == ErrorKind::HeadChanged => {
                Ok(CasOutcome::Conflict { current_etag: None })
            }
            Err(error) => Err(error.message()),
        }
    }

    async fn enumerate(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
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
        if raw != REPOSITORY_TAGS_PREFIX {
            return Err(
                "Mallard Cloud lists repositories through their tags only; objects are not enumerable"
                    .to_string(),
            );
        }
        let rows = self.listing(cursor.is_none()).await?;
        let mut entries = Vec::new();
        for row in &rows {
            if let Some((_, etag)) = Self::tag_bytes(row)? {
                let key = crate::storage_core::repository_tag_key(&BundleId::parse(
                    row.bundle_id.clone().unwrap_or_default(),
                )?)?;
                entries.push(StoreListEntry { key, etag });
            }
        }
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        let mut remaining = entries
            .into_iter()
            .filter(|entry| cursor.is_none_or(|cursor| entry.key.as_str() > cursor));
        let page: Vec<_> = remaining.by_ref().take(limit).collect();
        let next_cursor = if remaining.next().is_some() {
            page.last().map(|entry| entry.key.as_str().to_owned())
        } else {
            None
        };
        Ok(StoreListPage {
            entries: page,
            next_cursor,
        })
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
                // The commit record is the service's own; the manifest write
                // already carries everything it says.
                Key::Commit(_) | Key::Marker => outcomes.push(ImmutablePutOutcome::AlreadyPresent),
                Key::Library => return Err(CATALOG_REFUSAL.to_string()),
                Key::Upload(_) => return Err("Mallard Cloud has no probe namespace".to_string()),
                Key::Head(_) | Key::Tag(_) => {
                    return Err(format!("'{key}' is written by compare-and-swap only"))
                }
            }
        }
        Ok(outcomes)
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        match classify(key)? {
            Key::Head(bundle_id) => {
                self.inner
                    .runtime
                    .block_on(self.commit_head(&bundle_id, expectation, bytes))
            }
            // The website listing is the source of truth; tags are read
            // from it and never written.
            Key::Tag(_) => Ok(CasOutcome::Written {
                etag: "managed".to_string(),
            }),
            Key::Library => Err(CATALOG_REFUSAL.to_string()),
            _ => Err(format!("'{key}' is not a compare-and-swap object")),
        }
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        self.inner
            .runtime
            .block_on(self.enumerate(prefix, cursor, limit))
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
        assert_eq!(
            classify(&object("_head.json")).unwrap(),
            Key::Head(id.clone())
        );
        assert_eq!(
            classify(&object("_manifests/3-commit-abcdef12.json")).unwrap(),
            Key::Manifest {
                bundle_id: id.clone(),
                revision: 3,
                commit_id: "commit-abcdef12".into()
            }
        );
        assert_eq!(
            classify(&object("_commits/3-commit-abcdef12.json")).unwrap(),
            Key::Commit(id.clone())
        );
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
            Key::Upload(id.clone())
        );
        assert_eq!(
            classify(&crate::storage_core::repository_tag_key(&id).unwrap()).unwrap(),
            Key::Tag(id)
        );
        assert_eq!(
            classify(
                &crate::storage_core::library_object_key(
                    "library-0123456789abcdef0123456789abcdef",
                    "_head.json"
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
    use crate::bundle_engine::BundleEngine;
    use crate::managed_cloud::stub::StubManagedCloud;
    use crate::storage_core::{repository_object_key, HEAD_FILE};
    use mallard_domain::{
        BundleIdentity, OriginHostToken, OriginReplicaToken, PortableOrigin, SecretValue, StorageId,
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

    fn engine(store: ManagedObjectStore) -> BundleEngine<ManagedObjectStore> {
        BundleEngine::open(store, StorageId::parse("storage-cloud").unwrap()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinned_status_uses_one_head_read_and_never_recreates_a_missing_repository() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "status-owner").await;
        tokio::task::spawn_blocking({
            let store = store.clone();
            move || {
                engine(store)
                    .publish_registration(&identity(91), &origin(), 100)
                    .unwrap()
            }
        })
        .await
        .unwrap();
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
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, format!("/v1/repositories/{id}/head"));
        let observed = store.status(&bundle(91)).unwrap();
        assert_eq!(
            observed.state,
            super::super::client::ManagedCloudStatusState::Ready
        );
        assert_eq!(
            observed.link.unwrap().release,
            super::super::client::ReleaseState::NotPushed
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
        tokio::task::spawn_blocking({
            let owner = owner.clone();
            move || {
                engine(owner)
                    .publish_registration(&identity(92), &origin(), 100)
                    .unwrap()
            }
        })
        .await
        .unwrap();
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
    async fn registration_creates_the_repository_once_and_lists_it_as_a_tag() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let outcome = tokio::task::spawn_blocking({
            let store = store.clone();
            move || {
                let engine = engine(store);
                let first = engine
                    .publish_registration(&identity(1), &origin(), 100)
                    .unwrap();
                let second = engine
                    .publish_registration(&identity(1), &origin(), 200)
                    .unwrap();
                let listed = engine.list_remote_bundles().unwrap();
                (
                    first.published,
                    second.published,
                    second.snapshot.head.revision,
                    listed,
                )
            }
        })
        .await
        .unwrap();
        assert_eq!((outcome.0, outcome.1, outcome.2), (true, false, 1));
        assert_eq!(outcome.3.len(), 1);
        assert!(outcome.3[0].registration_only);
        assert_eq!(outcome.3[0].bundle_id, bundle(1));
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
        tokio::task::spawn_blocking(move || {
            let engine = engine(store);
            for number in [1, 2] {
                let mut requested = identity(number);
                requested.display_name = "My--Repo".into();
                requested.repository_fingerprint = Some("a".repeat(64));
                engine
                    .publish_registration(&requested, &origin(), 100)
                    .unwrap();
            }
        })
        .await
        .unwrap();
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

    fn next_manifest(previous: &BundleManifest) -> (Bytes, Bytes, BundleHead) {
        let mut manifest = previous.clone();
        manifest.revision += 1;
        manifest.commit_id = mallard_domain::generated_named_id("commit").unwrap();
        manifest.updated_at += 1;
        let bytes = Bytes::from(serde_json::to_vec(&manifest).unwrap());
        let head = BundleHead {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: manifest.bundle.bundle_id.clone(),
            kind: BundleKind::Project,
            revision: manifest.revision,
            commit_id: manifest.commit_id.clone(),
            manifest_key: format!(
                "_manifests/{}-{}.json",
                manifest.revision, manifest.commit_id
            ),
            manifest_sha256: sha256_hex(&bytes),
            manifest_size: bytes.len() as u64,
            updated_at: manifest.updated_at,
        };
        (bytes, Bytes::from_static(b"{}"), head)
    }

    /// Creates the upload for a manifest without committing it; returns its id.
    async fn open_upload(
        stub: &StubManagedCloud,
        store: &ManagedObjectStore,
        id: &BundleId,
        bytes: Bytes,
    ) -> String {
        let manifest: BundleManifest = serde_json::from_slice(&bytes).unwrap();
        let key = repository_object_key(
            id,
            &format!(
                "_manifests/{}-{}.json",
                manifest.revision, manifest.commit_id
            ),
        )
        .unwrap();
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

    /// Pushes a manifest as the next head on top of `parent`: the manifest and
    /// commit writes, then the head compare-and-swap.
    async fn push(
        store: &ManagedObjectStore,
        id: &BundleId,
        bytes: Bytes,
        head: BundleHead,
        parent: &str,
    ) -> CasOutcome {
        let manifest_key = repository_object_key(id, &head.manifest_key).unwrap();
        let commit_key = repository_object_key(
            id,
            &format!("_commits/{}-{}.json", head.revision, head.commit_id),
        )
        .unwrap();
        let head_key = repository_object_key(id, HEAD_FILE).unwrap();
        let expectation = CasExpectation::Match(parent.to_owned());
        let store = store.clone();
        tokio::task::spawn_blocking(move || {
            store
                .put_many(vec![
                    (manifest_key, bytes),
                    (commit_key, Bytes::from_static(b"{}")),
                ])
                .unwrap();
            store
                .compare_and_swap(&head_key, &expectation, &serde_json::to_vec(&head).unwrap())
                .unwrap()
        })
        .await
        .unwrap()
    }

    fn current_manifest(store: &ManagedObjectStore, number: u64) -> BundleManifest {
        let store = store.clone();
        tokio::task::block_in_place(move || {
            engine(store).inspect(&bundle(number)).unwrap().manifest
        })
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
        let registered = tokio::task::spawn_blocking({
            let store = store.clone();
            move || {
                engine(store)
                    .publish_registration(&identity(2), &origin(), 100)
                    .unwrap()
            }
        })
        .await
        .unwrap();
        let id = bundle(2);
        let base = registered.snapshot.manifest.clone();
        let base_commit = registered.snapshot.head.commit_id.clone();
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
        let (bytes2, _, head2) = next_manifest(&base);
        let same = open_upload(&stub, &store, &id, bytes2.clone()).await;
        let (other_bytes, _, _) = next_manifest(&base);
        let other = open_upload(&stub, &store, &id, other_bytes).await;
        assert_eq!(stub.state.lock().unwrap().uploads[&same].user_id, me);
        // A new process knows neither.
        store.inner.pending.lock().unwrap().clear();
        let written = push(&store, &id, bytes2, head2.clone(), &base_commit).await;
        assert!(matches!(written, CasOutcome::Written { .. }));
        {
            let state = stub.state.lock().unwrap();
            // The same-manifest leftover had to go first; the other is not stale yet and is left alone.
            assert_eq!(state.uploads[&same].state, "abandoned");
            assert_eq!(state.uploads[&other].state, "open");
        }
        assert_eq!(revision_of(&stub), 2);

        // The head moved, so the other leftover is stale: the next push ends it, after its own upload exists.
        let (bytes3, _, head3) = next_manifest(&current_manifest(&store, 2));
        let written = push(&store, &id, bytes3, head3.clone(), &head2.commit_id).await;
        assert!(matches!(written, CasOutcome::Written { .. }));
        assert_eq!(
            stub.state.lock().unwrap().uploads[&other].state,
            "abandoned"
        );
        assert!(created_before_ended(&stub, &other));
        assert_eq!(revision_of(&stub), 3);

        // A collaborator's open upload never blocks a push and is never touched.
        let (rika_store, rika) = store_for_member(&stub, "rika321", id.as_str()).await;
        let shared_base = current_manifest(&rika_store, 2);
        let (theirs_bytes, _, theirs_head) = next_manifest(&shared_base);
        let theirs = open_upload(&stub, &rika_store, &id, theirs_bytes).await;
        let (bytes4, _, head4) = next_manifest(&shared_base);
        let written = push(&store, &id, bytes4, head4.clone(), &head3.commit_id).await;
        assert!(matches!(written, CasOutcome::Written { .. }));
        {
            let state = stub.state.lock().unwrap();
            assert_eq!(state.uploads[&theirs].state, "open");
            assert_eq!(state.uploads[&theirs].user_id, rika);
        }

        // Their commit is the divergent one: a conflict that leaves the upload open...
        let conflict = tokio::task::spawn_blocking({
            let store = rika_store.clone();
            let head_key = repository_object_key(&id, HEAD_FILE).unwrap();
            let expectation = CasExpectation::Match(head3.commit_id.clone());
            move || {
                store
                    .compare_and_swap(
                        &head_key,
                        &expectation,
                        &serde_json::to_vec(&theirs_head).unwrap(),
                    )
                    .unwrap()
            }
        })
        .await
        .unwrap();
        assert_eq!(conflict, CasOutcome::Conflict { current_etag: None });
        assert_eq!(stub.state.lock().unwrap().uploads[&theirs].state, "open");
        // ...and their push on the moved head ends it only once the replacement exists.
        let (bytes5, _, head5) = next_manifest(&current_manifest(&rika_store, 2));
        let written = push(&rika_store, &id, bytes5, head5, &head4.commit_id).await;
        assert!(matches!(written, CasOutcome::Written { .. }));
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
    async fn a_moved_head_is_a_cas_conflict_that_leaves_the_upload_open() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let registered = tokio::task::spawn_blocking({
            let store = store.clone();
            move || {
                engine(store)
                    .publish_registration(&identity(3), &origin(), 100)
                    .unwrap()
            }
        })
        .await
        .unwrap();
        let id = bundle(3);
        let (manifest_bytes, commit_bytes, head) = next_manifest(&registered.snapshot.manifest);
        let manifest_key = repository_object_key(&id, &head.manifest_key).unwrap();
        let commit_key = repository_object_key(
            &id,
            &format!("_commits/{}-{}.json", head.revision, head.commit_id),
        )
        .unwrap();
        stub.state.lock().unwrap().fail_next_commit = true;
        let outcome = tokio::task::spawn_blocking({
            let store = store.clone();
            move || {
                store
                    .put_many(vec![
                        (manifest_key, manifest_bytes),
                        (commit_key, commit_bytes),
                    ])
                    .unwrap();
                store
                    .compare_and_swap(
                        &repository_object_key(&id, HEAD_FILE).unwrap(),
                        &CasExpectation::Match(registered.snapshot.head.commit_id.clone()),
                        &serde_json::to_vec(&head).unwrap(),
                    )
                    .unwrap()
            }
        })
        .await
        .unwrap();
        assert_eq!(outcome, CasOutcome::Conflict { current_etag: None });
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
            HEAD_FILE,
        )
        .unwrap();
        stub.state.lock().unwrap().revoke_token(&token);
        let (marker_read, head_read, health, library_read, library_write) =
            tokio::task::spawn_blocking(move || {
                let marker_read = store.get_metadata(&marker, 4096).unwrap();
                let head_read = store
                    .get_metadata(&repository_object_key(&bundle(4), HEAD_FILE).unwrap(), 4096);
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
