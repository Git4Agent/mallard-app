//! The managed store: the bundle engine's object-store contract answered by
//! the Mallard Cloud service, which is a gateway onto one R2 prefix per
//! repository laid out like every other Mallard storage (`STORE_API.md` in
//! the service tree; `PLAN_MANAGED_CLOUD_CAS_FREE.md` section 6.2). The
//! store is the R2 store's twin: it maps each engine key to the same key
//! under the repository's prefix, sends the same requests in the same
//! order, reports `supports_claims`, and gives its users the same finality.
//!
//! | Engine key | Behaviour |
//! | --- | --- |
//! | `.mallard/_storage.json` read | the constant marker, synthesized: a store has no repository before a bundle names one; the Worker wrote the same bytes into every prefix |
//! | `_objects/{sha}` write | staged in memory until the manifest arrives |
//! | `_manifests/…` write | resolve the repository, or create it from the manifest; build the views from the staged transcripts; `PUT` every staged object and view, then the manifest |
//! | `_commits/{bundle}/` listing | one listing request; the bundle's Cloud status is recorded from it |
//! | `_commits/` listing over every stream | refused: Mallard Cloud lists one repository at a time |
//! | `_claims/…` claim | one `PUT`, no transport retry: written or already present is `Mine`, taken is read back |
//! | `_commits/{bundle}/{record}` record | one `PUT`, no transport retry |
//! | reads | `GET`, streamed, digest verified by the shared body reader |
//! | `_uploads/…` | refused: probes are for user-owned stores |
//! | `.mallard/v1/library/{stream}/…` | every class above, routed to the catalog repository whose item stream is `{stream}` |
use super::client::{
    CreateCatalogItem, CreateRepository, Error, ErrorKind, ManagedCloudClient, ManagedCloudStatus,
    PutOutcome,
};
use super::views::build_views_for_manifest;
use crate::providers::AgentProviderRegistry;
use crate::storage_core::publication::MAX_RECORD_BYTES;
use crate::storage_core::{
    ClaimOutcome, ConnectionHealth, ExpectedObject, ImmutablePutOutcome, ObjectKey, ObjectPrefix,
    ObjectStore, StageOutcome, StorageMarker, StoreKeyPage, StoreListEntry, StoreListPage,
    StoredMetadata, VerifiedBody, LIBRARY_PREFIX, REPOSITORY_CLAIMS_PREFIX,
    REPOSITORY_COMMITS_PREFIX, REPOSITORY_PREFIX, STORAGE_MARKER_KEY, TAG_FILE,
};
use bytes::Bytes;
use mallard_domain::{sha256_hex, BundleId, BundleManifest};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::runtime::Handle;

/// The all-streams listing bundle discovery makes on a user-owned storage;
/// on Mallard Cloud discovery reads the repository list instead (decision
/// 10 of the plan).
pub const DISCOVERY_REFUSAL: &str = "Mallard Cloud lists one repository at a time.";

/// Which repository a key or prefix belongs to: a project bundle's, or a
/// catalog item stream's.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Project(BundleId),
    Library(String),
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    Marker,
    Object { scope: Scope, sha256: String },
    Manifest { scope: Scope },
    Record { scope: Scope },
    Claim { scope: Scope },
    Tag { stream: String },
    Upload,
}

impl Key {
    fn scope(&self) -> Option<&Scope> {
        match self {
            Self::Object { scope, .. }
            | Self::Manifest { scope }
            | Self::Record { scope }
            | Self::Claim { scope } => Some(scope),
            Self::Tag { .. } | Self::Marker | Self::Upload => None,
        }
    }
}

fn valid_library_stream(stream: &str) -> bool {
    stream
        .strip_prefix("library-")
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn classify(key: &ObjectKey) -> Result<Key, String> {
    let raw = key.as_str();
    if raw == STORAGE_MARKER_KEY {
        return Ok(Key::Marker);
    }
    if let Some(rest) = raw.strip_prefix(&format!("{LIBRARY_PREFIX}/")) {
        let (stream, relative) = rest
            .split_once('/')
            .ok_or_else(|| format!("unexpected Library key '{raw}'"))?;
        if !valid_library_stream(stream) {
            return Err(format!("unexpected Library key '{raw}'"));
        }
        let scope = Scope::Library(stream.to_owned());
        return if relative == TAG_FILE {
            Ok(Key::Tag {
                stream: stream.to_owned(),
            })
        } else if let Some(sha256) = relative.strip_prefix("_objects/") {
            mallard_domain::validate_sha256("managed object", sha256)?;
            Ok(Key::Object {
                scope,
                sha256: sha256.to_owned(),
            })
        } else if relative.starts_with("_manifests/") {
            Ok(Key::Manifest { scope })
        } else if relative.starts_with("_commits/") {
            Ok(Key::Record { scope })
        } else if relative.starts_with("_claims/") {
            Ok(Key::Claim { scope })
        } else {
            Err(format!("unexpected Library key '{raw}'"))
        };
    }
    for (prefix, class) in [
        (REPOSITORY_COMMITS_PREFIX, "record"),
        (REPOSITORY_CLAIMS_PREFIX, "claim"),
    ] {
        if let Some(rest) = raw.strip_prefix(&format!("{prefix}/")) {
            let (bundle, name) = rest
                .split_once('/')
                .ok_or_else(|| format!("unexpected {class} key '{raw}'"))?;
            if name.is_empty() || name.contains('/') {
                return Err(format!("unexpected {class} key '{raw}'"));
            }
            let scope = Scope::Project(BundleId::parse(bundle.to_string())?);
            return Ok(if class == "record" {
                Key::Record { scope }
            } else {
                Key::Claim { scope }
            });
        }
    }
    let rest = raw
        .strip_prefix(&format!("{REPOSITORY_PREFIX}/"))
        .ok_or_else(|| format!("object key is outside the managed namespaces: '{raw}'"))?;
    let (bundle, relative) = rest
        .split_once('/')
        .ok_or_else(|| format!("unexpected repository key '{raw}'"))?;
    let scope = Scope::Project(BundleId::parse(bundle.to_string())?);
    if relative.starts_with("_manifests/") {
        return Ok(Key::Manifest { scope });
    }
    if let Some(sha256) = relative.strip_prefix("_objects/") {
        mallard_domain::validate_sha256("managed object", sha256)?;
        return Ok(Key::Object {
            scope,
            sha256: sha256.to_owned(),
        });
    }
    if relative.starts_with("_uploads/") {
        return Ok(Key::Upload);
    }
    Err(format!("unexpected repository key '{raw}'"))
}

/// Where a listing prefix points: one repository's namespace, or the
/// all-streams listing Mallard Cloud refuses.
enum Listing {
    Scoped(Scope),
    Refused,
}

fn classify_prefix(prefix: &ObjectPrefix) -> Result<Listing, String> {
    let raw = prefix.as_str();
    if raw == REPOSITORY_PREFIX
        || raw == REPOSITORY_COMMITS_PREFIX
        || raw == REPOSITORY_CLAIMS_PREFIX
        || raw == LIBRARY_PREFIX
    {
        return Ok(Listing::Refused);
    }
    if let Some(rest) = raw.strip_prefix(&format!("{LIBRARY_PREFIX}/")) {
        let stream = rest.split('/').next().unwrap_or_default();
        if !valid_library_stream(stream) {
            return Err(format!("unexpected Library prefix '{raw}'"));
        }
        return Ok(Listing::Scoped(Scope::Library(stream.to_owned())));
    }
    for namespace in [
        REPOSITORY_COMMITS_PREFIX,
        REPOSITORY_CLAIMS_PREFIX,
        REPOSITORY_PREFIX,
    ] {
        if let Some(rest) = raw.strip_prefix(&format!("{namespace}/")) {
            let bundle = rest.split('/').next().unwrap_or_default();
            return Ok(Listing::Scoped(Scope::Project(BundleId::parse(
                bundle.to_string(),
            )?)));
        }
    }
    Err(format!("unexpected listing prefix '{raw}'"))
}

/// Whether a prefix is a bundle's own record stream: the one listing a
/// refresh makes, which carries the repository's status.
fn is_stream_prefix(prefix: &ObjectPrefix, scope: &Scope) -> bool {
    match scope {
        Scope::Project(bundle) => {
            prefix.as_str() == format!("{REPOSITORY_COMMITS_PREFIX}/{bundle}")
        }
        Scope::Library(stream) => prefix.as_str() == format!("{LIBRARY_PREFIX}/{stream}/_commits"),
    }
}

/// The facts of a Library manifest's one entry, from which the catalog
/// repository is created on its first push.
#[derive(Debug)]
struct LibraryItem {
    coordinate: CreateCatalogItem,
    display_name: String,
}

fn library_item(stream: &str, manifest_bytes: &[u8]) -> Result<LibraryItem, String> {
    let manifest: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|error| format!("parse the Library manifest being pushed: {error}"))?;
    if manifest["repository_id"].as_str() != Some(stream) {
        return Err("the Library manifest being pushed names another item stream".to_string());
    }
    let resources = manifest["resources"]
        .as_object()
        .ok_or("the Library manifest being pushed has no resources")?;
    if resources.len() != 1 {
        return Err(format!(
            "Mallard Cloud holds one item per repository; this Library manifest holds {}",
            resources.len()
        ));
    }
    let (resource_id, entry) = resources.iter().next().expect("one entry");
    let resource = &entry["resource"];
    let text = |value: &serde_json::Value, field: &str| -> Result<String, String> {
        value[field]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("the Library manifest being pushed lacks '{field}'"))
    };
    let resource_kind = text(resource, "resource_kind")?;
    let display_name = match resource_kind.as_str() {
        "agent_skill" => text(&resource["identity"], "effective_name")?,
        "plugin_install_intent" => text(&resource["identity"], "plugin_id")?,
        "desktop_pet" => text(&resource["descriptor"], "display_name")?,
        _ => resource_id.clone(),
    };
    Ok(LibraryItem {
        coordinate: CreateCatalogItem {
            provider: text(resource, "provider")?,
            resource_kind,
            format: text(resource, "format")?,
            format_version: resource["format_version"]
                .as_u64()
                .and_then(|version| u32::try_from(version).ok())
                .ok_or("the Library manifest being pushed lacks 'format_version'")?,
            resource_id: text(resource, "resource_id")?,
            library_repository_id: stream.to_owned(),
        },
        display_name,
    })
}

struct Inner {
    client: ManagedCloudClient,
    runtime: Handle,
    /// Scope → repository id, learned from lookups, listings and creates.
    repositories: Mutex<BTreeMap<Scope, String>>,
    /// Exact repository destinations from link metadata.
    pinned: Mutex<BTreeMap<BundleId, String>>,
    statuses: Mutex<BTreeMap<BundleId, ManagedCloudStatus>>,
    // Keep a superseded refresh from overwriting the next status observation.
    status_read: tokio::sync::Mutex<()>,
    /// Objects written before their manifest, per scope.
    staged: Mutex<BTreeMap<Scope, BTreeMap<String, Bytes>>>,
}

#[derive(Clone)]
pub struct ManagedObjectStore {
    inner: Arc<Inner>,
}

impl ManagedObjectStore {
    pub fn new(client: ManagedCloudClient, runtime: Handle) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                runtime,
                repositories: Mutex::new(BTreeMap::new()),
                pinned: Mutex::new(BTreeMap::new()),
                statuses: Mutex::new(BTreeMap::new()),
                status_read: tokio::sync::Mutex::new(()),
                staged: Mutex::new(BTreeMap::new()),
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
            self.remember(&Scope::Project(bundle.clone()), id)
        } else {
            if Self::lock(&self.inner.pinned)?.remove(bundle).is_some() {
                Self::lock(&self.inner.repositories)?.remove(&Scope::Project(bundle.clone()));
            }
            Self::lock(&self.inner.statuses)?.remove(bundle);
            Ok(())
        }
    }

    /// The Cloud status of a pinned bundle as its last stream listing
    /// reported it, or the connection's own state while it suspends the
    /// store.
    pub fn status(&self, bundle: &BundleId) -> Option<ManagedCloudStatus> {
        let repository_id = Self::lock(&self.inner.pinned).ok()?.get(bundle)?.clone();
        if let Some(health) = self
            .inner
            .client
            .health()
            .filter(|health| health.suspends())
        {
            return Some(ManagedCloudStatus {
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
            .block_on(self.repository_for(&Scope::Project(bundle.clone())))?
            .ok_or_else(|| {
                "Cloud repository is unavailable. Complete project setup before linking."
                    .to_string()
            })
    }

    pub fn client(&self) -> &ManagedCloudClient {
        &self.inner.client
    }

    /// The one request a Library refresh of one coordinate makes: the
    /// account's item repositories of that coordinate, each with its
    /// stream's names and the selected record and manifest inline. Served to
    /// the planning code only, never as a stream listing.
    pub fn library_coordinate(
        &self,
        query: &super::client::CoordinateQuery,
    ) -> Result<Vec<super::client::LibraryItemAnswer>, String> {
        let answers = self
            .inner
            .runtime
            .block_on(self.inner.client.library_coordinate(query))?;
        for answer in &answers {
            self.remember(
                &Scope::Library(answer.library_repository_id.clone()),
                &answer.repository_id,
            )?;
        }
        Ok(answers)
    }

    /// The account's project repositories with their indexed heads: what
    /// bundle discovery shows on Mallard Cloud instead of listing every
    /// stream (decision 10 of the plan). Display only: no publish or
    /// refresh depends on it.
    pub fn list_project_repositories(
        &self,
    ) -> Result<Vec<super::client::RepositorySummary>, String> {
        Ok(self
            .inner
            .runtime
            .block_on(self.inner.client.list_project_repositories())?)
    }

    fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
        mutex
            .lock()
            .map_err(|_| "the managed store's table is poisoned".to_string())
    }

    fn remember(&self, scope: &Scope, repository_id: &str) -> Result<(), String> {
        Self::lock(&self.inner.repositories)?.insert(scope.clone(), repository_id.to_owned());
        Ok(())
    }

    /// The repository id for a scope, from the pin, the table, or the
    /// service. `None` when the account knows no repository for it.
    async fn repository_for(&self, scope: &Scope) -> Result<Option<String>, String> {
        if let Scope::Project(bundle_id) = scope {
            if let Some(id) = Self::lock(&self.inner.pinned)?.get(bundle_id).cloned() {
                return Ok(Some(id));
            }
        }
        if let Some(id) = Self::lock(&self.inner.repositories)?.get(scope).cloned() {
            return Ok(Some(id));
        }
        let answer = match scope {
            Scope::Project(bundle_id) => {
                self.inner
                    .client
                    .repository_by_bundle(bundle_id.as_str())
                    .await?
            }
            Scope::Library(stream) => self.inner.client.repository_by_library(stream).await?,
        };
        if let Some(id) = &answer {
            self.remember(scope, id)?;
        }
        Ok(answer)
    }

    async fn download_into(
        &self,
        scope: &Scope,
        key: &ObjectKey,
        max: u64,
        sink: &mut impl Write,
    ) -> Result<Option<super::client::Downloaded>, String> {
        let Some(repository_id) = self.repository_for(scope).await? else {
            return Ok(None);
        };
        Ok(self
            .inner
            .client
            .get_object(&repository_id, key.as_str(), max, sink)
            .await?)
    }

    async fn stage_verified_async(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        let classified = classify(key)?;
        let scope = match &classified {
            Key::Object { scope, .. } | Key::Manifest { scope } => scope,
            _ => return Err(format!("'{key}' is not a verified object")),
        };
        mallard_domain::validate_sha256("expected object", &expected.sha256)?;
        let mut file = NamedTempFile::new().map_err(|error| format!("stage object: {error}"))?;
        let Some(downloaded) = self
            .download_into(scope, key, expected.size, &mut file)
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
        let classified = classify(key)?;
        let scope = match &classified {
            Key::Marker => {
                return Ok(Some(StoredMetadata {
                    bytes: serde_json::to_vec(&StorageMarker::current())
                        .map_err(|error| format!("encode storage marker: {error}"))?,
                    etag: "managed".to_string(),
                }))
            }
            Key::Upload => return Err("Mallard Cloud has no probe namespace".to_string()),
            Key::Tag { stream } => Scope::Library(stream.clone()),
            other => other.scope().expect("every other key has a scope").clone(),
        };
        let mut bytes = Vec::new();
        Ok(self
            .download_into(&scope, key, max_bytes, &mut bytes)
            .await?
            .map(|downloaded| StoredMetadata {
                bytes,
                etag: downloaded.etag,
            }))
    }

    /// One create-only write through the gateway, for a scope whose
    /// repository must already exist.
    async fn put_once(&self, scope: &Scope, key: &str, bytes: Bytes) -> Result<PutOutcome, String> {
        let repository_id = self.repository_for(scope).await?.ok_or_else(|| {
            "Cloud repository is unavailable. Push the item's manifest before its other objects."
                .to_string()
        })?;
        let sha256 = sha256_hex(&bytes);
        Ok(self
            .inner
            .client
            .put_object(&repository_id, key, bytes, &sha256)
            .await?)
    }

    /// `PUT` every transfer, `cloud_transfer_concurrency()` at a time. An
    /// object the repository already holds is fine; any other failure stops
    /// the rest.
    async fn transfer_all(
        &self,
        repository_id: &str,
        transfers: Vec<(String, Bytes)>,
    ) -> Result<(), String> {
        let concurrency = crate::app_settings::cloud_transfer_concurrency().max(1);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut workers = tokio::task::JoinSet::new();
        for (key, bytes) in transfers {
            let client = self.inner.client.clone();
            let repository_id = repository_id.to_owned();
            let semaphore = semaphore.clone();
            workers.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| Error::local("v1/repositories/{id}/store/{key}"))?;
                let sha256 = sha256_hex(&bytes);
                client
                    .put_object(&repository_id, &key, bytes, &sha256)
                    .await
            });
        }
        while let Some(joined) = workers.join_next().await {
            let outcome =
                joined.unwrap_or_else(|_| Err(Error::local("v1/repositories/{id}/store/{key}")));
            if let Err(error) = outcome {
                workers.abort_all();
                while workers.join_next().await.is_some() {}
                return Err(error.message());
            }
        }
        Ok(())
    }

    fn take_staged(&self, scope: &Scope) -> Result<BTreeMap<String, Bytes>, String> {
        Ok(Self::lock(&self.inner.staged)?
            .get(scope)
            .cloned()
            .unwrap_or_default())
    }

    fn clear_staged(&self, scope: &Scope) -> Result<(), String> {
        Self::lock(&self.inner.staged)?.remove(scope);
        Ok(())
    }

    /// The bundle manifest write: resolve or create the repository, build
    /// the views from the staged transcripts, put every staged object and
    /// every view, then the manifest. The record write follows through the
    /// shared protocol.
    async fn publish_manifest(
        &self,
        bundle_id: &BundleId,
        key: &ObjectKey,
        manifest_bytes: Bytes,
    ) -> Result<(), String> {
        let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("parse the manifest being pushed: {error}"))?;
        manifest.validate()?;
        if &manifest.bundle.bundle_id != bundle_id {
            return Err("the manifest being pushed names another bundle".to_string());
        }
        let scope = Scope::Project(bundle_id.clone());
        let repository_id = match self.repository_for(&scope).await? {
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
                    bundle_id: Some(bundle_id.as_str().to_owned()),
                    repository_fingerprint: manifest.bundle.repository_fingerprint.clone(),
                    catalog: None,
                };
                self.create(&scope, &spec).await?
            }
        };
        let staged = self.take_staged(&scope)?;
        let views =
            build_views_for_manifest(AgentProviderRegistry::production(), &manifest, &staged)?;
        // The objects first: the service admits a view only once the
        // transcript it was built from is stored.
        let objects = staged
            .iter()
            .map(|(sha256, bytes)| {
                (
                    format!("{REPOSITORY_PREFIX}/{bundle_id}/_objects/{sha256}"),
                    bytes.clone(),
                )
            })
            .collect();
        self.transfer_all(&repository_id, objects).await?;
        let views = views
            .iter()
            .map(|view| {
                (
                    format!(
                        "{REPOSITORY_PREFIX}/{bundle_id}/_views/{}-{}.json",
                        super::views::VIEW_SCHEMA,
                        view.source_sha256
                    ),
                    view.bytes.clone(),
                )
            })
            .collect();
        self.transfer_all(&repository_id, views).await?;
        let sha256 = sha256_hex(&manifest_bytes);
        self.inner
            .client
            .put_object(&repository_id, key.as_str(), manifest_bytes, &sha256)
            .await?;
        self.clear_staged(&scope)
    }

    /// The Library manifest write: resolve the item's repository, or create
    /// it from the manifest's one entry; put the staged payloads, then the
    /// manifest.
    async fn publish_library_manifest(
        &self,
        stream: &str,
        key: &ObjectKey,
        manifest_bytes: Bytes,
    ) -> Result<(), String> {
        let scope = Scope::Library(stream.to_owned());
        let repository_id = match self.repository_for(&scope).await? {
            Some(id) => id,
            None => {
                let item = library_item(stream, &manifest_bytes)?;
                let spec = CreateRepository {
                    repository_class: "catalog",
                    name: item.display_name,
                    bundle_id: None,
                    repository_fingerprint: None,
                    catalog: Some(item.coordinate),
                };
                self.create(&scope, &spec).await?
            }
        };
        let staged = self.take_staged(&scope)?;
        let transfers = staged
            .into_iter()
            .map(|(sha256, bytes)| {
                (
                    format!("{LIBRARY_PREFIX}/{stream}/_objects/{sha256}"),
                    bytes,
                )
            })
            .collect();
        self.transfer_all(&repository_id, transfers).await?;
        let sha256 = sha256_hex(&manifest_bytes);
        self.inner
            .client
            .put_object(&repository_id, key.as_str(), manifest_bytes, &sha256)
            .await?;
        self.clear_staged(&scope)
    }

    /// Create the scope's repository, or, having lost a race with another
    /// replica of this account, find the one it created.
    async fn create(&self, scope: &Scope, spec: &CreateRepository) -> Result<String, String> {
        match self.inner.client.create_repository(spec).await {
            Ok(created) => {
                self.remember(scope, &created.id)?;
                Ok(created.id)
            }
            Err(error) if error.kind == ErrorKind::AccountExists => {
                Self::lock(&self.inner.repositories)?.remove(scope);
                self.repository_for(scope)
                    .await?
                    .ok_or_else(|| error.message())
            }
            Err(error) => Err(error.message()),
        }
    }

    async fn put_record_async(
        &self,
        scope: &Scope,
        key: &ObjectKey,
        bytes: Bytes,
    ) -> Result<ImmutablePutOutcome, String> {
        match self.put_once(scope, key.as_str(), bytes).await? {
            PutOutcome::Written => Ok(ImmutablePutOutcome::Written),
            PutOutcome::AlreadyPresent => Ok(ImmutablePutOutcome::AlreadyPresent),
            PutOutcome::ClaimTaken => Err(format!("'{key}' is not a claim")),
        }
    }

    /// One create-only request at the claim key. Refused with these bytes
    /// already there is this writer's; refused with other bytes is read
    /// back, as the S3 store does.
    async fn put_claim_async(
        &self,
        scope: &Scope,
        key: &ObjectKey,
        bytes: Bytes,
    ) -> Result<ClaimOutcome, String> {
        match self.put_once(scope, key.as_str(), bytes).await? {
            PutOutcome::Written | PutOutcome::AlreadyPresent => Ok(ClaimOutcome::Mine),
            PutOutcome::ClaimTaken => {
                let mut found = Vec::new();
                match self
                    .download_into(scope, key, MAX_RECORD_BYTES, &mut found)
                    .await?
                {
                    Some(_) => Ok(ClaimOutcome::Taken(found)),
                    None => Err(format!(
                        "Mallard Cloud refused the claim '{key}' but the claim is missing"
                    )),
                }
            }
        }
    }

    async fn list_keys_async(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
    ) -> Result<StoreKeyPage, String> {
        let scope = match classify_prefix(prefix)? {
            Listing::Refused => return Err(DISCOVERY_REFUSAL.to_string()),
            Listing::Scoped(scope) => scope,
        };
        let stream = is_stream_prefix(prefix, &scope);
        let _status_read = if stream {
            Some(self.inner.status_read.lock().await)
        } else {
            None
        };
        let Some(repository_id) = self.repository_for(&scope).await? else {
            // A bundle the account has no repository for lists empty.
            return Ok(StoreKeyPage {
                keys: Vec::new(),
                next_cursor: None,
            });
        };
        let bundle = match &scope {
            Scope::Project(bundle) if stream => Some(bundle.clone()),
            _ => None,
        };
        let listing = match self
            .inner
            .client
            .list_keys(&repository_id, prefix.as_str(), cursor)
            .await
        {
            Ok(listing) => listing,
            Err(error) => {
                if let Some(bundle) = bundle {
                    let status = ManagedCloudStatus::failed(&repository_id, &error);
                    let message = status.message.clone().unwrap_or_else(|| error.message());
                    Self::lock(&self.inner.statuses)?.insert(bundle, status);
                    return Err(message);
                }
                return Err(error.message());
            }
        };
        if let Some(bundle) = bundle {
            Self::lock(&self.inner.statuses)?.insert(
                bundle,
                ManagedCloudStatus::observed(&repository_id, &listing),
            );
        }
        let keys = listing
            .keys
            .iter()
            .filter(|key| {
                key.strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|key| ObjectKey::parse(key.as_str()))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(StoreKeyPage {
            keys,
            next_cursor: listing.next_cursor,
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
                Key::Object { scope, sha256 } => {
                    if sha256_hex(&bytes) != sha256 {
                        return Err(format!("object '{key}' does not match its digest"));
                    }
                    Self::lock(&self.inner.staged)?
                        .entry(scope)
                        .or_default()
                        .insert(sha256, bytes);
                    outcomes.push(ImmutablePutOutcome::Written);
                }
                Key::Manifest {
                    scope: Scope::Project(bundle_id),
                } => {
                    self.inner
                        .runtime
                        .block_on(self.publish_manifest(&bundle_id, &key, bytes))?;
                    outcomes.push(ImmutablePutOutcome::Written);
                }
                Key::Manifest {
                    scope: Scope::Library(stream),
                } => {
                    self.inner
                        .runtime
                        .block_on(self.publish_library_manifest(&stream, &key, bytes))?;
                    outcomes.push(ImmutablePutOutcome::Written);
                }
                Key::Tag { stream } => {
                    let outcome = self.inner.runtime.block_on(self.put_once(
                        &Scope::Library(stream),
                        key.as_str(),
                        bytes,
                    ))?;
                    outcomes.push(match outcome {
                        PutOutcome::Written => ImmutablePutOutcome::Written,
                        PutOutcome::AlreadyPresent => ImmutablePutOutcome::AlreadyPresent,
                        PutOutcome::ClaimTaken => {
                            return Err(format!("'{key}' is not a claim"));
                        }
                    });
                }
                Key::Marker => outcomes.push(ImmutablePutOutcome::AlreadyPresent),
                Key::Upload => return Err("Mallard Cloud has no probe namespace".to_string()),
                Key::Record { .. } => {
                    return Err(format!("'{key}' is written as a publication record only"))
                }
                Key::Claim { .. } => return Err(format!("'{key}' is written as a claim only")),
            }
        }
        Ok(outcomes)
    }

    fn put_record(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        match classify(key)? {
            Key::Record { scope } => self.inner.runtime.block_on(self.put_record_async(
                &scope,
                key,
                Bytes::copy_from_slice(bytes),
            )),
            _ => Err(format!("'{key}' is not a publication record")),
        }
    }

    /// The service refuses a second write of a key in the same request,
    /// through R2's own create-only write.
    fn supports_claims(&self) -> bool {
        true
    }

    fn put_claim(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ClaimOutcome, String> {
        match classify(key)? {
            Key::Claim { scope } => self.inner.runtime.block_on(self.put_claim_async(
                &scope,
                key,
                Bytes::copy_from_slice(bytes),
            )),
            _ => Err(format!("'{key}' is not a claim")),
        }
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        let page = self.list_keys(prefix, cursor, limit)?;
        Ok(StoreListPage {
            entries: page
                .keys
                .into_iter()
                .map(|key| StoreListEntry {
                    key,
                    etag: String::new(),
                })
                .collect(),
            next_cursor: page.next_cursor,
        })
    }

    /// Nothing on Mallard Cloud is deleted from the desktop, so a listing
    /// never needs a revision: every listing is this one.
    fn list_keys(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreKeyPage, String> {
        if limit == 0 {
            return Err("listing limit must be positive".to_string());
        }
        self.inner
            .runtime
            .block_on(self.list_keys_async(prefix, cursor))
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
            classify(&object(&format!(
                "_manifests/3-commit-{}.json",
                "a".repeat(24)
            )))
            .unwrap(),
            Key::Manifest {
                scope: Scope::Project(id.clone())
            }
        );
        let record =
            ObjectKey::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{id}/record.json")).unwrap();
        assert_eq!(
            classify(&record).unwrap(),
            Key::Record {
                scope: Scope::Project(id.clone())
            }
        );
        let claim = ObjectKey::parse(format!("{REPOSITORY_CLAIMS_PREFIX}/{id}/root.json")).unwrap();
        assert_eq!(
            classify(&claim).unwrap(),
            Key::Claim {
                scope: Scope::Project(id.clone())
            }
        );
        let sha = "a".repeat(64);
        assert_eq!(
            classify(&object(&format!("_objects/{sha}"))).unwrap(),
            Key::Object {
                scope: Scope::Project(id.clone()),
                sha256: sha
            }
        );
        assert_eq!(classify(&object("_uploads/probe")).unwrap(), Key::Upload);
        let stream = "library-0123456789abcdef0123456789abcdef";
        let library =
            |relative: &str| crate::storage_core::library_object_key(stream, relative).unwrap();
        assert_eq!(
            classify(&library(TAG_FILE)).unwrap(),
            Key::Tag {
                stream: stream.into()
            }
        );
        assert_eq!(
            classify(&library("_commits/record.json")).unwrap(),
            Key::Record {
                scope: Scope::Library(stream.into())
            }
        );
        assert_eq!(
            classify(&library("_claims/root.json")).unwrap(),
            Key::Claim {
                scope: Scope::Library(stream.into())
            }
        );
        assert_eq!(
            classify(&library(&format!("_objects/{}", "b".repeat(64)))).unwrap(),
            Key::Object {
                scope: Scope::Library(stream.into()),
                sha256: "b".repeat(64)
            }
        );
    }

    #[test]
    fn prefixes_scope_to_one_repository_and_refuse_every_stream() {
        let id = bundle(9);
        let stream = ObjectPrefix::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{id}")).unwrap();
        assert!(matches!(
            classify_prefix(&stream).unwrap(),
            Listing::Scoped(Scope::Project(ref scoped)) if scoped == &id
        ));
        assert!(is_stream_prefix(&stream, &Scope::Project(id.clone())));
        let objects = ObjectPrefix::parse(format!("{REPOSITORY_PREFIX}/{id}/_objects")).unwrap();
        assert!(!is_stream_prefix(&objects, &Scope::Project(id.clone())));
        for refused in [
            REPOSITORY_PREFIX,
            REPOSITORY_COMMITS_PREFIX,
            REPOSITORY_CLAIMS_PREFIX,
            LIBRARY_PREFIX,
        ] {
            assert!(matches!(
                classify_prefix(&ObjectPrefix::parse(refused).unwrap()).unwrap(),
                Listing::Refused
            ));
        }
        let library = ObjectPrefix::parse(format!(
            "{LIBRARY_PREFIX}/library-0123456789abcdef0123456789abcdef/_commits"
        ))
        .unwrap();
        let scope = Scope::Library("library-0123456789abcdef0123456789abcdef".into());
        assert!(
            matches!(classify_prefix(&library).unwrap(), Listing::Scoped(ref scoped) if scoped == &scope)
        );
        assert!(is_stream_prefix(&library, &scope));
        assert!(classify_prefix(
            &ObjectPrefix::parse(format!("{LIBRARY_PREFIX}/catalog-x")).unwrap()
        )
        .is_err());
    }

    #[test]
    fn a_library_manifest_names_the_item_its_repository_is_created_from() {
        let stream = "library-0123456789abcdef0123456789abcdef";
        let manifest = serde_json::json!({
            "schema_version": 1, "repository_id": stream, "revision": 1, "commit_id": format!("commit-{}", "a".repeat(24)),
            "resources": {"resource-1": {"resource": {"resource_id": "resource-1", "provider": "codex", "resource_kind": "agent_skill",
                "format": "agentskills.io.skill", "format_version": 1, "identity": {"effective_name": "my-skill"}, "descriptor": {}}, "updated_at": 1}}
        });
        let item = library_item(stream, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(item.display_name, "my-skill");
        assert_eq!(item.coordinate.resource_kind, "agent_skill");
        assert_eq!(item.coordinate.library_repository_id, stream);
        let mut two = manifest.clone();
        two["resources"]["resource-2"] = two["resources"]["resource-1"].clone();
        assert!(library_item(stream, &serde_json::to_vec(&two).unwrap())
            .unwrap_err()
            .contains("one item per repository"));
        let mut other = manifest;
        other["repository_id"] = serde_json::json!("library-ffffffffffffffffffffffffffffffff");
        assert!(library_item(stream, &serde_json::to_vec(&other).unwrap()).is_err());
    }
}

#[cfg(test)]
mod stub_tests {
    use super::*;
    use crate::managed_cloud::client::{ManagedCloudStatusState, PublicationState, ReleaseState};
    use crate::managed_cloud::stub::{Fault, StubManagedCloud};
    use crate::storage_core::publication::{
        finish_claimed_push, list_stream, read_record, write_record, RecordStream, WriteOutcome,
        PUSH_UNFINISHED, REMOTE_CHANGED,
    };
    use crate::storage_core::{repository_object_key, ConfiguredObjectStore};
    use crate::storage_providers::StorageProviderRegistry;
    use mallard_domain::{
        BundleIdentity, BundleKind, OriginHostToken, OriginReplicaToken, PortableOrigin,
        PublicationToken, RecordName, SecretValue, BUNDLE_SCHEMA,
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

    /// A store for `login`, signed in at the stub, with the process cache
    /// a shared store has.
    async fn store(stub: &StubManagedCloud, login: &str) -> (ConfiguredObjectStore, String) {
        let (user, token) = stub.state.lock().unwrap().add_user(login);
        let mut secrets = mallard_domain::SecretMap::empty();
        secrets
            .insert(
                crate::storage_providers::managed_cloud::token_key(),
                SecretValue::parse(&token).unwrap(),
            )
            .unwrap();
        let config = mallard_domain::StorageProviderConfig::new(
            1,
            mallard_domain::BoundedConfigObject::encode(
                &crate::storage_providers::managed_cloud::ManagedCloudSettingsV1 {
                    service_origin: stub.endpoint.clone(),
                    account_id: user.id.clone(),
                    account_login: login.into(),
                },
            )
            .unwrap(),
            secrets,
        );
        let storage = mallard_domain::StorageConfig {
            id: mallard_domain::StorageId::parse(format!("storage-{}", user.id)).unwrap(),
            name: "Mallard Cloud".into(),
            provider: crate::storage_providers::managed_cloud::provider_key(),
            provider_config: config,
        };
        (
            ConfiguredObjectStore::open_with(StorageProviderRegistry::production(), &storage)
                .unwrap(),
            user.id,
        )
    }

    fn managed(store: &ConfiguredObjectStore) -> &ManagedObjectStore {
        store.managed_cloud().unwrap()
    }

    fn manifest(identity: BundleIdentity, revision: u64, updated_at: u64) -> BundleManifest {
        BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            revision,
            commit_id: mallard_domain::generated_named_id("commit").unwrap(),
            updated_at,
            bundle: identity,
            published_by_origin: origin(),
            scopes: BTreeMap::new(),
        }
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

    /// The publication record of `manifest` on `parent`, as the engine
    /// builds it, and its name.
    fn record_of(
        manifest: &BundleManifest,
        manifest_bytes: &[u8],
        parent: Option<&RecordName>,
        created_at_ms: u64,
    ) -> (RecordName, Vec<u8>) {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "bundle_id": manifest.bundle.bundle_id,
            "revision": manifest.revision,
            "commit_id": manifest.commit_id,
            "previous_commit_id": parent.map(|parent| parent.commit_id()),
            "previous_record_sha256": parent.map(|parent| parent.token().record_sha256()),
            "created_at_ms": created_at_ms,
            "manifest_key": format!("_manifests/{}-{}.json", manifest.revision, manifest.commit_id),
            "manifest_sha256": sha256_hex(manifest_bytes),
            "manifest_size": manifest_bytes.len(),
            "added_files": 0, "changed_files": 0, "removed_files": 0,
            "published_by_origin": manifest.published_by_origin,
            "summary": {"display_name": manifest.bundle.display_name, "kind": "project", "resources": 0, "files": 0,
                "registration_only": manifest.revision == 1, "providers": []},
        }))
        .unwrap();
        let name = RecordName::new(
            manifest.revision,
            PublicationToken::new(manifest.commit_id.clone(), sha256_hex(&bytes)).unwrap(),
            parent.map(|parent| parent.token().clone()),
            created_at_ms,
        )
        .unwrap();
        (name, bytes)
    }

    /// The engine's push: the manifest, then the record with list-then-write.
    fn push(
        store: &ConfiguredObjectStore,
        manifest: &BundleManifest,
        parent: Option<&RecordName>,
        created_at_ms: u64,
    ) -> Result<(RecordName, WriteOutcome), String> {
        let bytes = serde_json::to_vec(manifest).unwrap();
        store.put_immutable(&manifest_key(manifest), &bytes)?;
        let (name, record) = record_of(manifest, &bytes, parent, created_at_ms);
        let stream = RecordStream::project(&manifest.bundle.bundle_id)?;
        let outcome = write_record(store, &stream, &name, &record)?;
        Ok((name, outcome))
    }

    fn stored(outcome: Result<(RecordName, WriteOutcome), String>) -> RecordName {
        let (name, outcome) = outcome.unwrap();
        assert!(matches!(outcome, WriteOutcome::Stored(_)), "{outcome:?}");
        name
    }

    async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(work).await.unwrap()
    }

    fn repository_id(stub: &StubManagedCloud, number: u64) -> String {
        stub.state
            .lock()
            .unwrap()
            .repository_for_bundle(bundle(number).as_str())
            .unwrap()
            .id
            .clone()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_creates_the_repository_and_publishes_with_the_shared_protocol() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let first = manifest(identity(1), 1, 100);
        let name = blocking({
            let store = store.clone();
            let first = first.clone();
            move || stored(push(&store, &first, None, 1_000))
        })
        .await;
        let id = repository_id(&stub, 1);
        assert_eq!(
            stub.request_lines(),
            vec![
                format!("GET /v1/repositories/by-bundle/{}", bundle(1)),
                "POST /v1/repositories".to_string(),
                format!("PUT /v1/repositories/{id}/store/{}", manifest_key(&first)),
                format!(
                    "GET /v1/repositories/{id}/store?prefix={REPOSITORY_COMMITS_PREFIX}/{}",
                    bundle(1)
                ),
                format!(
                    "PUT /v1/repositories/{id}/store/{REPOSITORY_CLAIMS_PREFIX}/{}/root.json",
                    bundle(1)
                ),
                format!(
                    "PUT /v1/repositories/{id}/store/{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                    bundle(1),
                    name.file_name()
                ),
                format!(
                    "GET /v1/repositories/{id}/store?prefix={REPOSITORY_COMMITS_PREFIX}/{}",
                    bundle(1)
                ),
            ]
        );
        let state = stub.state.lock().unwrap();
        let repository = &state.repositories[&id];
        assert_eq!(repository.class, "non_git_project");
        assert!(repository.objects.contains_key(".mallard/_storage.json"));
        assert_eq!(
            repository.private_head.as_deref(),
            Some(name.file_name().as_str())
        );
        assert_eq!(repository.private_revision, 1);
        assert!(repository.publications[0].summary["registration_only"] == true);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_of_an_unchanged_bundle_makes_one_listing_request() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let first = manifest(identity(2), 1, 100);
        let (name, stream) = blocking({
            let store = store.clone();
            let first = first.clone();
            move || {
                let name = stored(push(&store, &first, None, 1_000));
                let stream = RecordStream::project(&first.bundle.bundle_id).unwrap();
                // The first refresh reads the record it has not seen.
                assert_eq!(
                    list_stream(&store, &stream).unwrap().mainline().selected(),
                    Some(&name)
                );
                (name, stream)
            }
        })
        .await;
        stub.clear_log();
        blocking({
            let store = store.clone();
            let stream = stream.clone();
            let name = name.clone();
            move || {
                let view = list_stream(&store, &stream).unwrap();
                assert_eq!(view.mainline().selected(), Some(&name));
                let bytes = read_record(&store, &stream, &name).unwrap();
                assert_eq!(sha256_hex(&bytes), name.token().record_sha256());
            }
        })
        .await;
        let id = repository_id(&stub, 2);
        assert_eq!(
            stub.request_lines(),
            vec![format!(
                "GET /v1/repositories/{id}/store?prefix={REPOSITORY_COMMITS_PREFIX}/{}",
                bundle(2)
            )]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_push_is_listing_claim_record_listing_after_the_manifest() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let first = manifest(identity(3), 1, 100);
        let second = manifest(identity(3), 2, 200);
        let root = blocking({
            let store = store.clone();
            let first = first.clone();
            move || stored(push(&store, &first, None, 1_000))
        })
        .await;
        stub.clear_log();
        let next = blocking({
            let store = store.clone();
            let second = second.clone();
            let root = root.clone();
            move || stored(push(&store, &second, Some(&root), 2_000))
        })
        .await;
        let id = repository_id(&stub, 3);
        let listing = format!(
            "GET /v1/repositories/{id}/store?prefix={REPOSITORY_COMMITS_PREFIX}/{}",
            bundle(3)
        );
        assert_eq!(
            stub.request_lines(),
            vec![
                format!("PUT /v1/repositories/{id}/store/{}", manifest_key(&second)),
                listing.clone(),
                format!(
                    "PUT /v1/repositories/{id}/store/{REPOSITORY_CLAIMS_PREFIX}/{}/{}.json",
                    bundle(3),
                    root.token()
                ),
                format!(
                    "PUT /v1/repositories/{id}/store/{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                    bundle(3),
                    next.file_name()
                ),
                listing,
            ]
        );
        let state = stub.state.lock().unwrap();
        let repository = &state.repositories[&id];
        assert_eq!(
            repository.private_head.as_deref(),
            Some(next.file_name().as_str())
        );
        assert_eq!(repository.private_revision, 2);
        assert_eq!(
            repository
                .publications
                .iter()
                .map(|p| p.state.as_str())
                .collect::<Vec<_>>(),
            ["retired", "available"]
        );
        // The claim of the root credits the pusher, and every object stays.
        assert_eq!(
            repository.objects[&format!("{REPOSITORY_CLAIMS_PREFIX}/{}/root.json", bundle(3))]
                .user_id
                .as_deref(),
            Some(repository.owner_id.as_str())
        );
        assert_eq!(
            repository
                .keys_under(&format!("{REPOSITORY_PREFIX}/{}/_manifests", bundle(3)))
                .len(),
            2
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_writers_on_one_version_the_claim_admits_one() {
        let stub = StubManagedCloud::start().await;
        let (owner, _) = store(&stub, "octocat").await;
        let (member, member_id) = store(&stub, "rika321").await;
        let base = manifest(identity(4), 1, 100);
        let root = blocking({
            let owner = owner.clone();
            let base = base.clone();
            move || stored(push(&owner, &base, None, 1_000))
        })
        .await;
        let id = repository_id(&stub, 4);
        stub.state
            .lock()
            .unwrap()
            .add_member(&id, &member_id, "member");
        managed(&member).set_pin(&bundle(4), Some(&id)).unwrap();
        let theirs = manifest(identity(4), 2, 200);
        let ours = manifest(identity(4), 2, 201);
        blocking({
            let owner = owner.clone();
            let member = member.clone();
            let root = root.clone();
            move || {
                // Both list the same childless head. The owner claims and
                // writes; the member's claim finds the owner's record.
                let winner = stored(push(&owner, &theirs, Some(&root), 2_000));
                let bytes = serde_json::to_vec(&ours).unwrap();
                member.put_immutable(&manifest_key(&ours), &bytes).unwrap();
                let (name, record) = record_of(&ours, &bytes, Some(&root), 2_001);
                let stream = RecordStream::project(&bundle(4)).unwrap();
                let claim = stream.claim_key(Some(root.token())).unwrap();
                let ClaimOutcome::Taken(found) = member.put_claim(&claim, &record).unwrap() else {
                    panic!("the member's claim was admitted");
                };
                assert_eq!(sha256_hex(&found), winner.token().record_sha256());
                // A record without its claim is refused by the Worker.
                let error = member
                    .put_record(&stream.key(&name).unwrap(), &record)
                    .unwrap_err();
                assert!(error.contains("claim_mismatch"), "{error}");
                // And the shared protocol, run whole, reads a changed storage.
                assert_eq!(
                    write_record(&member, &stream, &name, &record).unwrap_err(),
                    REMOTE_CHANGED
                );
                assert_eq!(
                    list_stream(&member, &stream).unwrap().mainline().selected(),
                    Some(&winner)
                );
            }
        })
        .await;
        let state = stub.state.lock().unwrap();
        assert_eq!(state.repositories[&id].publications.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_record_request_that_fails_after_its_claim_leaves_a_push_the_user_finishes() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let base = manifest(identity(5), 1, 100);
        let root = blocking({
            let store = store.clone();
            let base = base.clone();
            move || stored(push(&store, &base, None, 1_000))
        })
        .await;
        let id = repository_id(&stub, 5);
        stub.state.lock().unwrap().fail_next_record = Some(Fault::Before);
        let interrupted = manifest(identity(5), 2, 200);
        let (interrupted_name, stream) = blocking({
            let store = store.clone();
            let interrupted = interrupted.clone();
            let root = root.clone();
            move || {
                let bytes = serde_json::to_vec(&interrupted).unwrap();
                store
                    .put_immutable(&manifest_key(&interrupted), &bytes)
                    .unwrap();
                let (name, record) = record_of(&interrupted, &bytes, Some(&root), 2_000);
                let stream = RecordStream::project(&bundle(5)).unwrap();
                let error = write_record(&store, &stream, &name, &record).unwrap_err();
                assert!(error.contains("temporarily unavailable"), "{error}");
                // The claim landed, the record did not.
                assert!(list_stream(&store, &stream).unwrap().mainline().selected() == Some(&root));
                (name, stream)
            }
        })
        .await;
        {
            let state = stub.state.lock().unwrap();
            let repository = &state.repositories[&id];
            assert!(repository.objects.contains_key(&format!(
                "{REPOSITORY_CLAIMS_PREFIX}/{}/{}.json",
                bundle(5),
                root.token()
            )));
            assert!(!repository.objects.contains_key(&format!(
                "{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                bundle(5),
                interrupted_name.file_name()
            )));
        }
        // The next push onto that version, for other content, reads the
        // unfinished push; nothing writes the claimed record by itself.
        let other = manifest(identity(5), 2, 201);
        blocking({
            let store = store.clone();
            let stream = stream.clone();
            let root = root.clone();
            let interrupted_name = interrupted_name.clone();
            move || {
                let bytes = serde_json::to_vec(&other).unwrap();
                store.put_immutable(&manifest_key(&other), &bytes).unwrap();
                let (name, record) = record_of(&other, &bytes, Some(&root), 2_002);
                match write_record(&store, &stream, &name, &record).unwrap() {
                    WriteOutcome::Unfinished(claimed) => assert_eq!(claimed.name, interrupted_name),
                    WriteOutcome::Stored(_) => panic!("the unfinished push was overwritten"),
                }
                assert_eq!(
                    write_record(&store, &stream, &name, &record)
                        .unwrap()
                        .stored()
                        .unwrap_err(),
                    PUSH_UNFINISHED
                );
            }
        })
        .await;
        stub.clear_log();
        blocking({
            let store = store.clone();
            let stream = stream.clone();
            let interrupted_name = interrupted_name.clone();
            move || {
                let view = finish_claimed_push(&store, &stream).unwrap();
                assert_eq!(view.mainline().selected(), Some(&interrupted_name));
            }
        })
        .await;
        let record_puts = stub
            .request_lines()
            .into_iter()
            .filter(|line| line.starts_with("PUT") && line.contains("/_commits/"))
            .count();
        assert_eq!(record_puts, 1);
        let state = stub.state.lock().unwrap();
        let repository = &state.repositories[&id];
        assert_eq!(
            repository.private_head.as_deref(),
            Some(interrupted_name.file_name().as_str())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn discard_reads_as_cleared_and_the_next_push_restarts_without_reuploading() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let first = manifest(identity(6), 1, 100);
        let second = manifest(identity(6), 2, 200);
        blocking({
            let store = store.clone();
            let first = first.clone();
            let second = second.clone();
            move || {
                let root = stored(push(&store, &first, None, 1_000));
                stored(push(&store, &second, Some(&root), 2_000));
            }
        })
        .await;
        let id = repository_id(&stub, 6);
        let objects_before = stub.state.lock().unwrap().repositories[&id].objects.len();
        stub.state.lock().unwrap().discard(&id);
        {
            let state = stub.state.lock().unwrap();
            let repository = &state.repositories[&id];
            assert!(repository.private_head.is_none());
            assert!(repository.publications.iter().all(|p| p.state == "removed"));
            // Two claims and two records went; the marker and both manifests stay.
            assert_eq!(repository.objects.len(), objects_before - 4);
        }
        let restart = manifest(identity(6), 1, 300);
        blocking({
            let store = store.clone();
            let restart = restart.clone();
            move || {
                let stream = RecordStream::project(&bundle(6)).unwrap();
                assert!(list_stream(&store, &stream).unwrap().mainline().is_empty());
                stored(push(&store, &restart, None, 3_000));
            }
        })
        .await;
        let state = stub.state.lock().unwrap();
        let repository = &state.repositories[&id];
        assert_eq!(repository.private_revision, 3);
        assert_eq!(
            repository
                .publication(repository.private_head.as_deref().unwrap())
                .unwrap()
                .manifest_revision,
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_store_never_reads_the_index() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let first = manifest(identity(7), 1, 100);
        let second = manifest(identity(7), 2, 200);
        let root = blocking({
            let store = store.clone();
            let first = first.clone();
            move || stored(push(&store, &first, None, 1_000))
        })
        .await;
        let id = repository_id(&stub, 7);
        // The index lags behind the next record, and the head pointer is
        // moved by hand: the desktop selects from the listing all the same.
        stub.state.lock().unwrap().skip_next_index = true;
        let next = blocking({
            let store = store.clone();
            let second = second.clone();
            let root = root.clone();
            move || stored(push(&store, &second, Some(&root), 2_000))
        })
        .await;
        stub.state.lock().unwrap().misplace_head(&id, None);
        let third = manifest(identity(7), 3, 300);
        blocking({
            let store = store.clone();
            let next = next.clone();
            move || {
                let stream = RecordStream::project(&bundle(7)).unwrap();
                assert_eq!(
                    list_stream(&store, &stream).unwrap().mainline().selected(),
                    Some(&next)
                );
                stored(push(&store, &third, Some(&next), 3_000));
            }
        })
        .await;
        let state = stub.state.lock().unwrap();
        assert_eq!(state.repositories[&id].publications.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn discovery_is_refused_and_a_bundle_without_a_repository_lists_empty() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        blocking({
            let store = store.clone();
            move || {
                let every_stream = ObjectPrefix::parse(REPOSITORY_COMMITS_PREFIX).unwrap();
                assert_eq!(
                    store.list_keys(&every_stream, None, 10).unwrap_err(),
                    DISCOVERY_REFUSAL
                );
                let stream = RecordStream::project(&bundle(8)).unwrap();
                assert!(list_stream(&store, &stream).unwrap().mainline().is_empty());
                assert!(crate::storage_core::publication::list_project_streams(&store).is_err());
            }
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pinned_status_comes_from_the_stream_listing() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let first = manifest(identity(9), 1, 100);
        let second = manifest(identity(9), 2, 200);
        let root = blocking({
            let store = store.clone();
            let first = first.clone();
            move || stored(push(&store, &first, None, 1_000))
        })
        .await;
        let id = repository_id(&stub, 9);
        managed(&store).set_pin(&bundle(9), Some(&id)).unwrap();
        let stream = RecordStream::project(&bundle(9)).unwrap();
        let refresh = |store: ConfiguredObjectStore, stream: RecordStream| async move {
            blocking(move || list_stream(&store, &stream)).await
        };
        refresh(store.clone(), stream.clone()).await.unwrap();
        let registered = managed(&store).status(&bundle(9)).unwrap();
        assert_eq!(registered.state, ManagedCloudStatusState::Ready);
        assert_eq!(
            registered.link.as_ref().unwrap().release,
            ReleaseState::NotPushed
        );
        assert_eq!(
            registered.publication.as_ref().unwrap().state,
            PublicationState::NoDraft
        );
        assert_eq!(
            registered.link.as_ref().unwrap().website_url,
            format!("{}/octocat/Cloud-project", stub.endpoint)
        );

        blocking({
            let store = store.clone();
            let second = second.clone();
            move || stored(push(&store, &second, Some(&root), 2_000))
        })
        .await;
        let pushed = managed(&store).status(&bundle(9)).unwrap();
        assert_eq!(pushed.link.as_ref().unwrap().release, ReleaseState::Private);
        let publication = pushed.publication.as_ref().unwrap();
        assert_eq!(publication.state, PublicationState::NotPublic);
        assert_eq!(publication.private_revision, Some(2));

        stub.state.lock().unwrap().release(&id, None);
        refresh(store.clone(), stream.clone()).await.unwrap();
        let released = managed(&store).status(&bundle(9)).unwrap();
        assert_eq!(
            released.link.as_ref().unwrap().release,
            ReleaseState::Public
        );
        assert_eq!(
            released.publication.as_ref().unwrap().state,
            PublicationState::MatchesPublished
        );

        stub.state.lock().unwrap().fail_next_listing = true;
        assert!(refresh(store.clone(), stream.clone()).await.is_err());
        assert_eq!(
            managed(&store).status(&bundle(9)).unwrap().state,
            ManagedCloudStatusState::Unavailable
        );
        refresh(store.clone(), stream.clone()).await.unwrap();
        assert_eq!(
            managed(&store).status(&bundle(9)).unwrap().state,
            ManagedCloudStatusState::Ready
        );

        stub.state.lock().unwrap().repositories.remove(&id);
        let error = refresh(store.clone(), stream.clone()).await.unwrap_err();
        assert!(
            error.contains("deleted or you no longer have access"),
            "{error}"
        );
        assert_eq!(
            managed(&store).status(&bundle(9)).unwrap().state,
            ManagedCloudStatusState::Inaccessible
        );
        // Removing the pin disables the status, even with a warm store.
        managed(&store).set_pin(&bundle(9), None).unwrap();
        assert!(managed(&store).status(&bundle(9)).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn library_keys_route_to_the_item_repository() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let stream_id = format!("library-{}", "3".repeat(32));
        let resource_id = format!("resource-{}", "4".repeat(64));
        let skill = b"---\nname: my-skill\n---\n".to_vec();
        let skill_sha = sha256_hex(&skill);
        let commit = mallard_domain::generated_named_id("commit").unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1, "repository_id": stream_id, "revision": 1, "commit_id": commit, "updated_at": 100,
            "captured_with": {"app": "mallard", "version": "test"},
            "published_by_origin": {"origin_host_token": "a".repeat(64), "origin_replica_token": "b".repeat(64)},
            "resources": {resource_id.clone(): {"resource": {"resource_id": resource_id, "provider": "codex", "resource_kind": "agent_skill",
                "format": "agentskills.io.skill", "format_version": 1, "identity": {"root": "provider-home.skills", "effective_name": "my-skill", "install_directory": "my-skill"},
                "descriptor": {}, "content_sha256": "0".repeat(64), "files": [{"path": "SKILL.md", "sha256": skill_sha, "size": skill.len(), "mode": 420}]}, "updated_at": 100}}
        });
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let tag = serde_json::to_vec(&serde_json::json!({"schema_version": 1, "repository_id": stream_id, "provider": "codex",
            "resource_kind": "agent_skill", "format": "agentskills.io.skill", "format_version": 1})).unwrap();
        let record = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "repository_id": stream_id, "revision": 1, "commit_id": commit, "created_at_ms": 1_000,
            "manifest_key": format!("_manifests/1-{commit}.json"), "manifest_sha256": sha256_hex(&manifest_bytes), "manifest_size": manifest_bytes.len(),
            "published_by_origin": {"origin_host_token": "a".repeat(64), "origin_replica_token": "b".repeat(64)},
        })).unwrap();
        let name = RecordName::new(
            1,
            PublicationToken::new(commit.clone(), sha256_hex(&record)).unwrap(),
            None,
            1_000,
        )
        .unwrap();
        blocking({
            let store = store.clone();
            let stream_id = stream_id.clone();
            let name = name.clone();
            move || {
                let key = |relative: &str| {
                    crate::storage_core::library_object_key(&stream_id, relative).unwrap()
                };
                let stream = RecordStream::library(&stream_id).unwrap();
                // Nothing yet: the account has no repository for the item.
                assert!(list_stream(&store, &stream).unwrap().mainline().is_empty());
                assert!(store.get_metadata(&key(TAG_FILE), 4096).unwrap().is_none());
                store
                    .put_immutable(&key(&format!("_objects/{skill_sha}")), &skill)
                    .unwrap();
                store
                    .put_immutable(
                        &key(&format!("_manifests/1-{commit}.json")),
                        &manifest_bytes,
                    )
                    .unwrap();
                assert_eq!(
                    store.put_immutable(&key(TAG_FILE), &tag).unwrap(),
                    ImmutablePutOutcome::Written
                );
                assert_eq!(
                    store.put_immutable(&key(TAG_FILE), &tag).unwrap(),
                    ImmutablePutOutcome::AlreadyPresent
                );
                let view = write_record(&store, &stream, &name, &record)
                    .unwrap()
                    .stored()
                    .unwrap();
                assert_eq!(view.mainline().selected(), Some(&name));
                assert_eq!(
                    store
                        .get_metadata(&key(TAG_FILE), 4096)
                        .unwrap()
                        .unwrap()
                        .bytes,
                    tag
                );
            }
        })
        .await;
        let state = stub.state.lock().unwrap();
        let repository = state.repository_for_library(&stream_id).unwrap();
        assert_eq!(repository.class, "catalog");
        assert_eq!(repository.name, "my-skill");
        assert_eq!(
            repository.catalog.as_ref().unwrap().resource_id,
            resource_id
        );
        assert_eq!(
            repository.private_head.as_deref(),
            Some(name.file_name().as_str())
        );
        assert_eq!(repository.publications[0].summary["kind"], "catalog");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_revoked_session_suspends_the_store() {
        let stub = StubManagedCloud::start().await;
        let (user, token) = stub.state.lock().unwrap().add_user("octocat");
        let client =
            ManagedCloudClient::new(&stub.endpoint, Some(SecretValue::parse(&token).unwrap()))
                .unwrap();
        let store = ManagedObjectStore::new(client, Handle::current());
        let marker = ObjectKey::parse(STORAGE_MARKER_KEY).unwrap();
        stub.state.lock().unwrap().revoke_token(&token);
        let _ = user;
        let (marker_read, listing, health) = blocking(move || {
            let marker_read = store.get_metadata(&marker, 4096).unwrap();
            let stream =
                ObjectPrefix::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{}", bundle(4))).unwrap();
            let listing = store.list_keys(&stream, None, 10);
            let health = store.connection_health().unwrap();
            (marker_read, listing, health)
        })
        .await;
        assert!(marker_read.is_some(), "the marker needs no request");
        assert!(listing.unwrap_err().contains("Reconnect"));
        assert_eq!(
            health.state,
            crate::storage_core::ConnectionState::ReconnectRequired
        );
        assert!(health.suspends());
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
