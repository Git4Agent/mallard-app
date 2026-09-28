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
//! | `_commits/{bundle}/` listing | one listing request; the bundle's Cloud status is recorded from it. A repository that answers gone lists empty, or as the repository this account collaborates on in its place |
//! | `_commits/` listing over every stream | refused: Mallard Cloud lists one repository at a time |
//! | `_claims/…` claim | one `PUT`, no transport retry: written or already present is `Mine`, taken is read back |
//! | `_commits/{bundle}/{record}` record | one `PUT`, no transport retry |
//! | reads | `GET`, streamed, digest verified by the shared body reader |
//! | `_uploads/…` | refused: probes are for user-owned stores |
//! | `.mallard/v1/library/{stream}/…` | every class above, routed to the catalog repository whose item stream is `{stream}` |
//!
//! Each Push fixes the observed repository revision for all its writes.
//! Discard blocks writes, then rejects that revision after deletion finishes.
use super::client::{
    CreateCatalogItem, CreateRepository, Error, ErrorKind, ManagedCloudClient, ManagedCloudStatus,
    PutOutcome, StoreKeyListing,
};
use super::views::build_views_for_manifest;
use crate::providers::AgentProviderRegistry;
use crate::storage_core::publication::{MAX_RECORD_BYTES, REMOTE_CHANGED};
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
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// The scopes whose repository answered gone at their last stream
    /// listing: deleted, being deleted, or out of this account's reach. Each
    /// holds the repository this account collaborates on that is read in its
    /// place, if there is one. A pinned bundle stays here until its pin
    /// answers again or is replaced; any other scope only while nothing
    /// stands in, because what stands in is what the table learns.
    moved: Mutex<BTreeMap<Scope, Option<String>>>,
    statuses: Mutex<BTreeMap<BundleId, ManagedCloudStatus>>,
    // Keep a superseded refresh from overwriting the next status observation.
    status_read: tokio::sync::Mutex<()>,
    /// Objects written before their manifest, per scope.
    staged: Mutex<BTreeMap<Scope, BTreeMap<String, Bytes>>>,
    /// Latest listing revision, and the revision fixed for an active Push.
    revisions: Mutex<BTreeMap<String, u64>>,
    write_revisions: Mutex<BTreeMap<Scope, u64>>,
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
                moved: Mutex::new(BTreeMap::new()),
                statuses: Mutex::new(BTreeMap::new()),
                status_read: tokio::sync::Mutex::new(()),
                staged: Mutex::new(BTreeMap::new()),
                revisions: Mutex::new(BTreeMap::new()),
                write_revisions: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// The link's saved repository, set at every engine open. The same pin
    /// again changes nothing: a pin that answered gone stays gone, with what
    /// stands in for it, until a Push or a Pull has saved another.
    pub fn set_pin(&self, bundle: &BundleId, id: Option<&str>) -> Result<(), String> {
        let scope = Scope::Project(bundle.clone());
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
                Self::lock(&self.inner.moved)?.remove(&scope);
            }
            self.remember(&scope, id)
        } else {
            if Self::lock(&self.inner.pinned)?.remove(bundle).is_some() {
                Self::lock(&self.inner.repositories)?.remove(&scope);
                Self::lock(&self.inner.moved)?.remove(&scope);
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

    /// The repository a bundle's operation read or wrote, for the link to
    /// save as its pin: what stands in for a pin that answered gone, else
    /// the pin, else what the store learned. `None` when the bundle has no
    /// repository. No request is made.
    pub fn repository_in_use(&self, bundle: &BundleId) -> Result<Option<String>, String> {
        let scope = Scope::Project(bundle.clone());
        if let Some(standing) = Self::lock(&self.inner.moved)?.get(&scope).cloned() {
            return Ok(standing);
        }
        if let Some(id) = Self::lock(&self.inner.pinned)?.get(bundle).cloned() {
            return Ok(Some(id));
        }
        Ok(Self::lock(&self.inner.repositories)?.get(&scope).cloned())
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
        // The answer holds this account's own item repositories, so each is
        // one it collaborates on.
        for answer in &answers {
            let scope = Scope::Library(answer.library_repository_id.clone());
            Self::lock(&self.inner.moved)?.remove(&scope);
            self.remember(&scope, &answer.repository_id)?;
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

    /// The repository id for a scope: what stands in for one that answered
    /// gone, else the pin, the table, or the service. `None` when the
    /// account knows no repository for it, or its repository is gone and
    /// nothing stands in; a gone scope makes no request here.
    async fn repository_for(&self, scope: &Scope) -> Result<Option<String>, String> {
        if let Some(standing) = Self::lock(&self.inner.moved)?.get(scope).cloned() {
            return Ok(standing);
        }
        if let Scope::Project(bundle_id) = scope {
            if let Some(id) = Self::lock(&self.inner.pinned)?.get(bundle_id).cloned() {
                return Ok(Some(id));
            }
        }
        if let Some(id) = Self::lock(&self.inner.repositories)?.get(scope).cloned() {
            return Ok(Some(id));
        }
        let answer = self.ask_service(scope).await?;
        if let Some(id) = &answer {
            self.remember(scope, id)?;
        }
        Ok(answer)
    }

    /// The repository the service resolves a scope to: this account's own
    /// first, else an open public one.
    async fn ask_service(&self, scope: &Scope) -> Result<Option<String>, Error> {
        match scope {
            Scope::Project(bundle_id) => {
                self.inner
                    .client
                    .repository_by_bundle(bundle_id.as_str())
                    .await
            }
            Scope::Library(stream) => self.inner.client.repository_by_library(stream).await,
        }
    }

    fn pin_of(&self, scope: &Scope) -> Result<Option<String>, String> {
        match scope {
            Scope::Project(bundle) => Ok(Self::lock(&self.inner.pinned)?.get(bundle).cloned()),
            Scope::Library(_) => Ok(None),
        }
    }

    /// Record what a stream listing told about a pinned bundle. The status
    /// names the pin, whichever repository was read.
    fn record_status(
        &self,
        scope: &Scope,
        status: impl FnOnce(&str) -> ManagedCloudStatus,
    ) -> Result<(), String> {
        if let (Scope::Project(bundle), Some(pin)) = (scope, self.pin_of(scope)?) {
            Self::lock(&self.inner.statuses)?.insert(bundle.clone(), status(&pin));
        }
        Ok(())
    }

    /// A stream listing that failed for any reason but a gone repository:
    /// the pinned bundle's status says why, and the listing is an error.
    fn listing_failed(&self, scope: &Scope, error: &Error) -> Result<String, String> {
        let mut message = error.message();
        self.record_status(scope, |pin| {
            let status = ManagedCloudStatus::failed(pin, error);
            if let Some(sentence) = &status.message {
                message.clone_from(sentence);
            }
            status
        })?;
        Ok(message)
    }

    /// The first page of a scope's record stream: the listing every
    /// refresh, Push and Pull begins with, and where a repository that is
    /// gone is met. `None` reads as an empty stream.
    ///
    /// The scope's own repository is asked first, a pin every time. When it
    /// answers `not_found` or `repository_removed` the stream is read from
    /// the repository that stands in for it, or from the one the service
    /// resolves the scope to, and only when that listing states a role for
    /// this account: another computer of the account created it again. A
    /// public repository the account does not belong to is never read in
    /// its place. With nothing to read the stream is empty, and the next
    /// manifest write creates the repository. No other answer reads as
    /// empty.
    async fn list_stream_head(
        &self,
        scope: &Scope,
        prefix: &ObjectPrefix,
    ) -> Result<Option<StoreKeyListing>, String> {
        let pin = self.pin_of(scope)?;
        let was_gone = Self::lock(&self.inner.moved)?.contains_key(scope);
        let own = match &pin {
            Some(pin) => Some(pin.clone()),
            None if was_gone => None,
            None => self.repository_for(scope).await?,
        };
        let gone = match &own {
            Some(id) => match self.inner.client.list_keys(id, prefix.as_str(), None).await {
                Ok(listing) => {
                    Self::lock(&self.inner.moved)?.remove(scope);
                    self.record_status(scope, |pin| ManagedCloudStatus::observed(pin, &listing))?;
                    return Ok(Some(listing));
                }
                Err(error) if error.repository_gone() => Some(error),
                Err(error) => return Err(self.listing_failed(scope, &error)?),
            },
            // A scope the account has no repository for lists empty.
            None if !was_gone => return Ok(None),
            None => None,
        };
        if pin.is_none() {
            Self::lock(&self.inner.repositories)?.remove(scope);
        }
        let standing = Self::lock(&self.inner.moved)?.get(scope).cloned().flatten();
        let candidate = match standing {
            Some(id) => Some(id),
            None => match self.ask_service(scope).await {
                Ok(answer) => answer,
                Err(error) => return Err(self.listing_failed(scope, &error)?),
            },
        };
        let mut adopted = None;
        if let Some(id) = candidate {
            match self
                .inner
                .client
                .list_keys(&id, prefix.as_str(), None)
                .await
            {
                Ok(listing)
                    if listing
                        .link
                        .as_ref()
                        .is_some_and(|link| link.role.is_some()) =>
                {
                    adopted = Some((id, listing));
                }
                Ok(_) => {}
                Err(error) if error.repository_gone() => {}
                Err(error) => return Err(self.listing_failed(scope, &error)?),
            }
        }
        match adopted {
            Some((id, listing)) => {
                if pin.is_some() {
                    Self::lock(&self.inner.moved)?.insert(scope.clone(), Some(id));
                    self.record_status(scope, |pin| ManagedCloudStatus::observed(pin, &listing))?;
                } else {
                    Self::lock(&self.inner.moved)?.remove(scope);
                    self.remember(scope, &id)?;
                }
                Ok(Some(listing))
            }
            None => {
                Self::lock(&self.inner.moved)?.insert(scope.clone(), None);
                if let Some(error) = &gone {
                    self.record_status(scope, |pin| ManagedCloudStatus::failed(pin, error))?;
                }
                Ok(None)
            }
        }
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

    /// Freeze before the first write; later confirming listings cannot renew it.
    async fn write_revision(&self, scope: &Scope, repository_id: &str) -> Result<u64, String> {
        if let Some(revision) = Self::lock(&self.inner.write_revisions)?.get(scope).copied() {
            return Ok(revision);
        }
        let observed = Self::lock(&self.inner.revisions)?
            .get(repository_id)
            .copied();
        let revision = match observed {
            Some(revision) => revision,
            None => {
                let prefix = match scope {
                    Scope::Project(bundle) => format!("{REPOSITORY_COMMITS_PREFIX}/{bundle}"),
                    Scope::Library(stream) => format!("{LIBRARY_PREFIX}/{stream}/_commits"),
                };
                self.inner
                    .client
                    .list_keys(repository_id, &prefix, None)
                    .await?
                    .link
                    .ok_or_else(|| "Mallard Cloud omitted the repository revision".to_string())?
                    .repository_revision
            }
        };
        Self::lock(&self.inner.write_revisions)?.insert(scope.clone(), revision);
        Ok(revision)
    }

    async fn put_once(&self, scope: &Scope, key: &str, bytes: Bytes) -> Result<PutOutcome, String> {
        let repository_id = self.repository_for(scope).await?.ok_or_else(|| {
            "Cloud repository is unavailable. Push the item's manifest before its other objects."
                .to_string()
        })?;
        let revision = self.write_revision(scope, &repository_id).await?;
        let sha256 = sha256_hex(&bytes);
        Ok(self
            .inner
            .client
            .put_object(&repository_id, key, bytes, &sha256, revision)
            .await?)
    }

    /// `PUT` every transfer, `cloud_transfer_concurrency()` at a time. An
    /// object the repository already holds is fine. On failure, stop queued
    /// transfers and wait for active requests to finish.
    async fn transfer_all(
        &self,
        repository_id: &str,
        transfers: Vec<(String, Bytes)>,
        revision: u64,
    ) -> Result<(), String> {
        let concurrency = crate::app_settings::cloud_transfer_concurrency().max(1);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut workers = tokio::task::JoinSet::new();
        let stopped = Arc::new(AtomicBool::new(false));
        for (key, bytes) in transfers {
            let client = self.inner.client.clone();
            let repository_id = repository_id.to_owned();
            let semaphore = semaphore.clone();
            let stopped = stopped.clone();
            workers.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| Error::local("v1/repositories/{id}/store/{key}"))?;
                if stopped.load(Ordering::Acquire) {
                    return Ok(());
                }
                let sha256 = sha256_hex(&bytes);
                let result = client
                    .put_object(&repository_id, &key, bytes, &sha256, revision)
                    .await
                    .map(|_| ());
                if result.is_err() {
                    stopped.store(true, Ordering::Release);
                }
                result
            });
        }
        let mut first_error = None;
        while let Some(joined) = workers.join_next().await {
            let outcome =
                joined.unwrap_or_else(|_| Err(Error::local("v1/repositories/{id}/store/{key}")));
            if let Err(error) = outcome {
                stopped.store(true, Ordering::Release);
                first_error.get_or_insert_with(|| error.message());
            }
        }
        // Keep active requests alive until the Worker releases their admission.
        first_error.map_or(Ok(()), Err)
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
        if !Self::lock(&self.inner.staged)?.contains_key(&scope) {
            Self::lock(&self.inner.write_revisions)?.remove(&scope);
        }
        let revision = self.write_revision(&scope, &repository_id).await?;
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
        self.transfer_all(&repository_id, objects, revision).await?;
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
        self.transfer_all(&repository_id, views, revision).await?;
        let sha256 = sha256_hex(&manifest_bytes);
        self.inner
            .client
            .put_object(
                &repository_id,
                key.as_str(),
                manifest_bytes,
                &sha256,
                revision,
            )
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
        if !Self::lock(&self.inner.staged)?.contains_key(&scope) {
            Self::lock(&self.inner.write_revisions)?.remove(&scope);
        }
        let revision = self.write_revision(&scope, &repository_id).await?;
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
        self.transfer_all(&repository_id, transfers, revision)
            .await?;
        let sha256 = sha256_hex(&manifest_bytes);
        self.inner
            .client
            .put_object(
                &repository_id,
                key.as_str(),
                manifest_bytes,
                &sha256,
                revision,
            )
            .await?;
        self.clear_staged(&scope)
    }

    /// Create the scope's repository, or, having lost a race with another
    /// replica of this account, find the one it created. After a gone
    /// answer the new repository stands in for the old one; a create the
    /// service refuses then means another computer created it since the
    /// listing, which is a storage that changed under the Push.
    async fn create(&self, scope: &Scope, spec: &CreateRepository) -> Result<String, String> {
        let was_gone = Self::lock(&self.inner.moved)?.contains_key(scope);
        match self.inner.client.create_repository(spec).await {
            Ok(created) => {
                Self::lock(&self.inner.revisions)?.insert(created.id.clone(), created.revision);
                if was_gone && self.pin_of(scope)?.is_some() {
                    Self::lock(&self.inner.moved)?.insert(scope.clone(), Some(created.id.clone()));
                } else {
                    Self::lock(&self.inner.moved)?.remove(scope);
                    self.remember(scope, &created.id)?;
                }
                Ok(created.id)
            }
            Err(error) if error.kind == ErrorKind::AccountExists && was_gone => {
                Err(REMOTE_CHANGED.to_string())
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
        let outcome = self.put_once(scope, key.as_str(), bytes).await;
        Self::lock(&self.inner.write_revisions)?.remove(scope);
        match outcome? {
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
        let listing = if stream && cursor.is_none() {
            self.list_stream_head(&scope, prefix).await?
        } else {
            match self.repository_for(&scope).await? {
                Some(repository_id) => match self
                    .inner
                    .client
                    .list_keys(&repository_id, prefix.as_str(), cursor)
                    .await
                {
                    Ok(listing) => {
                        if stream {
                            self.record_status(&scope, |pin| {
                                ManagedCloudStatus::observed(pin, &listing)
                            })?;
                        }
                        Some(listing)
                    }
                    Err(error) if stream => return Err(self.listing_failed(&scope, &error)?),
                    Err(error) => return Err(error.message()),
                },
                None => None,
            }
        };
        let Some(listing) = listing else {
            // A scope with no repository, or whose repository is gone, lists empty.
            return Ok(StoreKeyPage {
                keys: Vec::new(),
                next_cursor: None,
            });
        };
        if let Some(link) = &listing.link {
            Self::lock(&self.inner.revisions)?
                .insert(link.repository_id.clone(), link.repository_revision);
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
                    if !Self::lock(&self.inner.staged)?.contains_key(&scope) {
                        Self::lock(&self.inner.write_revisions)?.remove(&scope);
                        if let Some(repository_id) =
                            self.inner.runtime.block_on(self.repository_for(&scope))?
                        {
                            self.inner
                                .runtime
                                .block_on(self.write_revision(&scope, &repository_id))?;
                        }
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
                    let result = self
                        .inner
                        .runtime
                        .block_on(self.publish_manifest(&bundle_id, &key, bytes));
                    if result.is_err() {
                        let scope = Scope::Project(bundle_id);
                        self.clear_staged(&scope)?;
                        Self::lock(&self.inner.write_revisions)?.remove(&scope);
                    }
                    result?;
                    outcomes.push(ImmutablePutOutcome::Written);
                }
                Key::Manifest {
                    scope: Scope::Library(stream),
                } => {
                    let result = self
                        .inner
                        .runtime
                        .block_on(self.publish_library_manifest(&stream, &key, bytes));
                    if result.is_err() {
                        let scope = Scope::Library(stream);
                        self.clear_staged(&scope)?;
                        Self::lock(&self.inner.write_revisions)?.remove(&scope);
                    }
                    result?;
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

    /// Read fresh keys and retain the repository revision for the next Push.
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_transfer_stops_queued_writes_and_drains_active_writes() {
        use http_body_util::{BodyExt, Full};
        use hyper::{body::Incoming, server::conn::http1, service::service_fn};
        use hyper_util::rt::TokioIo;
        use std::sync::atomic::AtomicUsize;
        use std::time::Duration;

        let width = crate::app_settings::cloud_transfer_concurrency();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let started = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(tokio::sync::Barrier::new(width + 1));
        let (release, held) = tokio::sync::watch::channel(false);
        let server = {
            let started = started.clone();
            let ready = ready.clone();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let started = started.clone();
                    let ready = ready.clone();
                    let held = held.clone();
                    tokio::spawn(async move {
                        let service = service_fn(move |request: hyper::Request<Incoming>| {
                            let started = started.clone();
                            let ready = ready.clone();
                            let mut held = held.clone();
                            async move {
                                let index = started.fetch_add(1, Ordering::SeqCst);
                                assert_eq!(request.headers()["x-mallard-repository-revision"], "7");
                                request.into_body().collect().await.unwrap();
                                if index < width {
                                    ready.wait().await;
                                }
                                let (status, body) = if index == 0 {
                                    (503, r#"{"error":"temporarily_unavailable"}"#)
                                } else {
                                    held.wait_for(|released| *released).await.unwrap();
                                    (200, r#"{"stored":true}"#)
                                };
                                Ok::<_, std::convert::Infallible>(
                                    hyper::Response::builder()
                                        .status(status)
                                        .body(Full::new(Bytes::from_static(body.as_bytes())))
                                        .unwrap(),
                                )
                            }
                        });
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            })
        };
        let client = ManagedCloudClient::new(
            &endpoint,
            Some(mallard_domain::SecretValue::parse("fixture-transfer-token").unwrap()),
        )
        .unwrap();
        let store = ManagedObjectStore::new(client, Handle::current());
        let transfers = (0..width * 2)
            .map(|index| (format!("_objects/{index}"), Bytes::from_static(b"fixture")))
            .collect();
        let mut transfer =
            tokio::spawn(async move { store.transfer_all("repository", transfers, 7).await });
        tokio::time::timeout(Duration::from_secs(10), ready.wait())
            .await
            .unwrap();
        let early = tokio::time::timeout(Duration::from_millis(100), &mut transfer).await;
        let admitted = started.load(Ordering::SeqCst);
        release.send(true).unwrap();
        assert!(
            early.is_err(),
            "an active upload was canceled after its peer failed"
        );
        let error = tokio::time::timeout(Duration::from_secs(10), transfer)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        server.abort();
        assert!(error.contains("temporarily_unavailable"), "{error}");
        assert_eq!(admitted, width);
        assert_eq!(started.load(Ordering::SeqCst), width);
    }

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
    use crate::managed_cloud::stub::{Fault, ListingFault, StubManagedCloud};
    use crate::storage_core::publication::{
        finish_claimed_push, list_stream, read_record, write_record, RecordStream, StreamView,
        WriteOutcome, PUSH_UNFINISHED,
    };
    use crate::storage_core::{repository_object_key, ConfiguredObjectStore};
    use crate::storage_providers::StorageProviderRegistry;
    use mallard_domain::{
        BundleIdentity, BundleKind, OriginHostToken, OriginReplicaToken, PortableOrigin,
        PublicationToken, RecordName, SecretValue, BUNDLE_SCHEMA,
    };
    use std::collections::BTreeSet;

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

    /// What a refresh does: one listing of the bundle's stream, selected.
    async fn refresh(store: &ConfiguredObjectStore, number: u64) -> Result<StreamView, String> {
        let store = store.clone();
        blocking(move || list_stream(&store, &RecordStream::project(&bundle(number)).unwrap()))
            .await
    }

    /// Push `revisions` versions of bundle `number` in a row and pin the
    /// repository the first one created, as a link does. The names pushed.
    async fn pushed(
        stub: &StubManagedCloud,
        store: &ConfiguredObjectStore,
        number: u64,
        revisions: u64,
    ) -> (String, Vec<RecordName>) {
        let names = blocking({
            let store = store.clone();
            move || {
                let mut names = Vec::<RecordName>::new();
                for revision in 1..=revisions {
                    let next = manifest(identity(number), revision, revision * 100);
                    names.push(stored(push(&store, &next, names.last(), revision * 1_000)));
                }
                names
            }
        })
        .await;
        let id = repository_id(stub, number);
        managed(store).set_pin(&bundle(number), Some(&id)).unwrap();
        (id, names)
    }

    fn stream_listing(id: &str, number: u64) -> String {
        format!(
            "GET /v1/repositories/{id}/store?prefix={REPOSITORY_COMMITS_PREFIX}/{}",
            bundle(number)
        )
    }

    fn by_bundle(number: u64) -> String {
        format!("GET /v1/repositories/by-bundle/{}", bundle(number))
    }

    /// The requests that wrote, and the repositories they named.
    fn written_to(stub: &StubManagedCloud) -> BTreeSet<String> {
        stub.requests()
            .iter()
            .filter(|request| request.method == "PUT")
            .filter_map(|request| request.path.split('/').nth(3).map(str::to_owned))
            .collect()
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
    async fn discard_deletes_content_and_the_next_push_starts_fresh() {
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
        // A private repository: the discard keeps no push.
        assert_eq!(stub.state.lock().unwrap().discard(&id), Ok(None));
        {
            let state = stub.state.lock().unwrap();
            let repository = &state.repositories[&id];
            assert!(repository.private_head.is_none());
            assert!(repository.publications.iter().all(|p| p.state == "removed"));
            // Only the storage marker remains.
            assert_eq!(repository.objects.len(), 1);
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

        stub.state.lock().unwrap().fail_next_listing = Some(ListingFault::Unavailable);
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
        // Removing the pin disables the status, even with a warm store.
        managed(&store).set_pin(&bundle(9), None).unwrap();
        assert!(managed(&store).status(&bundle(9)).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pinned_repository_that_is_gone_lists_empty_and_a_push_creates_it_again() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let (old, _) = pushed(&stub, &store, 20, 2).await;
        {
            let mut state = stub.state.lock().unwrap();
            state.remove(&old);
            state.finish_removal(&old);
        }
        stub.clear_log();
        assert!(refresh(&store, 20).await.unwrap().mainline().is_empty());
        let status = managed(&store).status(&bundle(20)).unwrap();
        assert_eq!(status.state, ManagedCloudStatusState::Gone);
        assert_eq!(status.repository_id, old);
        assert!(status.link.is_none() && status.publication.is_none());
        assert_eq!(
            status.message.as_deref(),
            Some("Deleted, or this account lost access. Push creates it again.")
        );
        // The pin, then the service. The two records this process had seen
        // are dropped without a request: the stream has no repository.
        assert_eq!(
            stub.request_lines(),
            vec![stream_listing(&old, 20), by_bundle(20)]
        );
        assert_eq!(
            managed(&store).repository_in_use(&bundle(20)).unwrap(),
            None
        );
        assert!(blocking({
            let store = store.clone();
            move || managed(&store).resolve_repository(&bundle(20))
        })
        .await
        .is_err());

        // A Push starts a first version. Its manifest write creates the
        // repository, and every write lands in it.
        stub.clear_log();
        let again = manifest(identity(20), 1, 300);
        let name = blocking({
            let store = store.clone();
            let again = again.clone();
            move || stored(push(&store, &again, None, 3_000))
        })
        .await;
        let new = repository_id(&stub, 20);
        assert_ne!(new, old);
        assert_eq!(
            stub.request_lines(),
            vec![
                "POST /v1/repositories".to_string(),
                format!("PUT /v1/repositories/{new}/store/{}", manifest_key(&again)),
                // Each listing asks the pin first, then reads what stands in for it.
                stream_listing(&old, 20),
                stream_listing(&new, 20),
                format!(
                    "PUT /v1/repositories/{new}/store/{REPOSITORY_CLAIMS_PREFIX}/{}/root.json",
                    bundle(20)
                ),
                format!(
                    "PUT /v1/repositories/{new}/store/{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                    bundle(20),
                    name.file_name()
                ),
                stream_listing(&old, 20),
                stream_listing(&new, 20),
            ]
        );
        assert_eq!(written_to(&stub), BTreeSet::from([new.clone()]));
        assert_eq!(
            managed(&store).repository_in_use(&bundle(20)).unwrap(),
            Some(new.clone())
        );
        let status = managed(&store).status(&bundle(20)).unwrap();
        assert_eq!(status.state, ManagedCloudStatusState::Ready);
        // The status names the pin; its link names the repository that was read.
        assert_eq!(status.repository_id, old);
        let link = status.link.unwrap();
        assert_eq!(link.repository_id, new);
        assert_eq!(link.role.as_deref(), Some("owner"));
        // The address is the old one: the name was free.
        assert_eq!(
            link.website_url,
            format!("{}/octocat/Cloud-project", stub.endpoint)
        );
        {
            let state = stub.state.lock().unwrap();
            assert_eq!(state.repositories[&old].state, "removed");
            assert_eq!(state.repositories[&new].name, "Cloud-project");
            assert_eq!(
                state.repositories[&new].private_head.as_deref(),
                Some(name.file_name().as_str())
            );
        }
        // The link saves the new repository as its pin: the old one is asked no more.
        managed(&store).set_pin(&bundle(20), Some(&new)).unwrap();
        stub.clear_log();
        assert_eq!(
            refresh(&store, 20).await.unwrap().mainline().selected(),
            Some(&name)
        );
        assert_eq!(stub.request_lines(), vec![stream_listing(&new, 20)]);
        assert_eq!(
            managed(&store).status(&bundle(20)).unwrap().repository_id,
            new
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pinned_repository_being_deleted_lists_empty_and_a_push_creates_it_again() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let (old, _) = pushed(&stub, &store, 21, 1).await;
        stub.state.lock().unwrap().remove(&old);
        assert!(refresh(&store, 21).await.unwrap().mainline().is_empty());
        let status = managed(&store).status(&bundle(21)).unwrap();
        assert_eq!(status.state, ManagedCloudStatusState::Deleting);
        assert_eq!(status.message.as_deref(), Some("Push creates it again."));
        // The old repository is still being deleted when the Push lands.
        let again = manifest(identity(21), 1, 300);
        let name = blocking({
            let store = store.clone();
            move || stored(push(&store, &again, None, 3_000))
        })
        .await;
        let new = repository_id(&stub, 21);
        assert_ne!(new, old);
        {
            let state = stub.state.lock().unwrap();
            assert_eq!(state.repositories[&old].state, "deleting");
            assert_eq!(state.repositories[&new].name, "Cloud-project");
            assert!(!state.repositories[&old].objects.is_empty());
        }
        // The worker erases the old prefix; the new repository is untouched.
        stub.state.lock().unwrap().finish_removal(&old);
        assert_eq!(
            refresh(&store, 21).await.unwrap().mainline().selected(),
            Some(&name)
        );
        assert_eq!(
            managed(&store).status(&bundle(21)).unwrap().state,
            ManagedCloudStatusState::Ready
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_computer_with_the_old_pin_reads_the_repository_the_first_created() {
        let stub = StubManagedCloud::start().await;
        // Two computers of one account: two stores, one session each.
        let (first, _) = store(&stub, "octocat").await;
        let second = {
            let token = {
                let state = stub.state.lock().unwrap();
                let user = state.users.values().next().unwrap().clone();
                drop(state);
                stub.state.lock().unwrap().add_session(&user.id)
            };
            let client =
                ManagedCloudClient::new(&stub.endpoint, Some(SecretValue::parse(&token).unwrap()))
                    .unwrap();
            ManagedObjectStore::new(client, Handle::current())
        };
        let (old, _) = pushed(&stub, &first, 22, 1).await;
        second.set_pin(&bundle(22), Some(&old)).unwrap();
        stub.state.lock().unwrap().remove(&old);
        assert!(refresh(&first, 22).await.unwrap().mainline().is_empty());
        let again = manifest(identity(22), 1, 300);
        let name = blocking({
            let first = first.clone();
            move || stored(push(&first, &again, None, 3_000))
        })
        .await;
        let new = repository_id(&stub, 22);

        stub.clear_log();
        let stream =
            ObjectPrefix::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{}", bundle(22))).unwrap();
        let page = blocking({
            let second = second.clone();
            let stream = stream.clone();
            move || second.list_keys(&stream, None, 1_000)
        })
        .await
        .unwrap();
        assert_eq!(
            page.keys
                .iter()
                .map(|key| key.as_str().to_owned())
                .collect::<Vec<_>>(),
            vec![format!(
                "{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                bundle(22),
                name.file_name()
            )]
        );
        assert_eq!(
            stub.request_lines(),
            vec![
                stream_listing(&old, 22),
                by_bundle(22),
                stream_listing(&new, 22)
            ]
        );
        let status = second.status(&bundle(22)).unwrap();
        assert_eq!(status.state, ManagedCloudStatusState::Ready);
        assert_eq!(status.repository_id, old);
        assert_eq!(status.link.unwrap().repository_id, new);
        assert_eq!(
            second.repository_in_use(&bundle(22)).unwrap(),
            Some(new.clone())
        );
        // Reads follow: the record comes from the repository that stands in.
        let record = blocking({
            let second = second.clone();
            let key = RecordStream::project(&bundle(22))
                .unwrap()
                .key(&name)
                .unwrap();
            move || second.get_metadata(&key, MAX_RECORD_BYTES)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(sha256_hex(&record.bytes), name.token().record_sha256());
        // The next listing asks the pin again and the service no more.
        stub.clear_log();
        blocking({
            let second = second.clone();
            move || second.list_keys(&stream, None, 1_000)
        })
        .await
        .unwrap();
        assert_eq!(
            stub.request_lines(),
            vec![stream_listing(&old, 22), stream_listing(&new, 22)]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gone_pin_never_follows_a_public_repository_of_another_account() {
        let stub = StubManagedCloud::start().await;
        let (tom, _) = store(&stub, "tom").await;
        let (rika, rika_id) = store(&stub, "rika").await;
        let owned_by_rika = |stub: &StubManagedCloud| {
            stub.state
                .lock()
                .unwrap()
                .repositories
                .values()
                .find(|repository| repository.state == "active" && repository.owner_id == rika_id)
                .cloned()
        };
        // Tom's private repository of the project. Rika is a Member, and her
        // link pins it.
        let (theirs, _) = pushed(&stub, &tom, 23, 2).await;
        stub.state
            .lock()
            .unwrap()
            .add_member(&theirs, &rika_id, "member");
        managed(&rika).set_pin(&bundle(23), Some(&theirs)).unwrap();
        assert!(!refresh(&rika, 23).await.unwrap().mainline().is_empty());
        // Tom removes her: lost access reads as gone, and her Push creates
        // a private repository on her own account.
        stub.state
            .lock()
            .unwrap()
            .repositories
            .get_mut(&theirs)
            .unwrap()
            .members
            .remove(&rika_id);
        assert!(refresh(&rika, 23).await.unwrap().mainline().is_empty());
        assert_eq!(
            managed(&rika).status(&bundle(23)).unwrap().state,
            ManagedCloudStatusState::Gone
        );
        blocking({
            let rika = rika.clone();
            move || stored(push(&rika, &manifest(identity(23), 1, 300), None, 3_000))
        })
        .await;
        let own = owned_by_rika(&stub).unwrap();
        assert_eq!(own.visibility, "private");
        assert_eq!(
            managed(&rika).repository_in_use(&bundle(23)).unwrap(),
            Some(own.id.clone())
        );
        managed(&rika).set_pin(&bundle(23), Some(&own.id)).unwrap();

        // Later Tom's repository becomes public, and later still Rika
        // removes her own on the website.
        {
            let mut state = stub.state.lock().unwrap();
            state.release(&theirs, None);
            state.remove(&own.id);
        }
        let theirs_before = stub.state.lock().unwrap().repositories[&theirs].clone();
        stub.clear_log();
        assert!(refresh(&rika, 23).await.unwrap().mainline().is_empty());
        // The service resolves the project to Tom's public repository. Its
        // listing states no role for her, so it is not read in place of hers.
        assert_eq!(
            stub.request_lines(),
            vec![
                stream_listing(&own.id, 23),
                by_bundle(23),
                stream_listing(&theirs, 23)
            ]
        );
        let status = managed(&rika).status(&bundle(23)).unwrap();
        assert_eq!(status.state, ManagedCloudStatusState::Deleting);
        assert!(status.link.is_none());
        assert_eq!(managed(&rika).repository_in_use(&bundle(23)).unwrap(), None);

        // Her Push creates her own again, private, and writes nothing to Tom's.
        stub.clear_log();
        blocking({
            let rika = rika.clone();
            move || stored(push(&rika, &manifest(identity(23), 1, 400), None, 4_000))
        })
        .await;
        let again = owned_by_rika(&stub).unwrap();
        assert_ne!(again.id, own.id);
        assert_eq!(again.visibility, "private");
        assert_eq!(again.name, own.name);
        assert_eq!(written_to(&stub), BTreeSet::from([again.id.clone()]));
        let state = stub.state.lock().unwrap();
        assert_eq!(
            state.repositories[&theirs].objects.len(),
            theirs_before.objects.len()
        );
        assert_eq!(
            state.repositories[&theirs].private_head,
            theirs_before.private_head
        );
        drop(state);
        // Tom's repository was asked for nothing but the listings that read its role.
        let asked = stub
            .request_lines()
            .into_iter()
            .filter(|line| line.contains(&theirs))
            .collect::<BTreeSet<_>>();
        assert!(asked.is_subset(&BTreeSet::from([stream_listing(&theirs, 23)])));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pin_that_answers_again_is_read_again() {
        let stub = StubManagedCloud::start().await;
        let (owner, _) = store(&stub, "tom").await;
        let (member, member_id) = store(&stub, "rika").await;
        let (id, names) = pushed(&stub, &owner, 24, 2).await;
        stub.state
            .lock()
            .unwrap()
            .add_member(&id, &member_id, "member");
        managed(&member).set_pin(&bundle(24), Some(&id)).unwrap();
        assert_eq!(
            refresh(&member, 24).await.unwrap().mainline().selected(),
            names.last()
        );
        // Access is lost: the pin reads as gone, the stream as empty.
        let membership = stub
            .state
            .lock()
            .unwrap()
            .repositories
            .get_mut(&id)
            .unwrap()
            .members
            .remove(&member_id)
            .unwrap();
        assert!(refresh(&member, 24).await.unwrap().mainline().is_empty());
        assert_eq!(
            managed(&member).status(&bundle(24)).unwrap().state,
            ManagedCloudStatusState::Gone
        );
        assert_eq!(
            managed(&member).repository_in_use(&bundle(24)).unwrap(),
            None
        );
        // The same pin at the next engine open changes nothing.
        managed(&member).set_pin(&bundle(24), Some(&id)).unwrap();
        assert_eq!(
            managed(&member).repository_in_use(&bundle(24)).unwrap(),
            None
        );
        // Access comes back: the pin answers, and is what is read.
        stub.state
            .lock()
            .unwrap()
            .repositories
            .get_mut(&id)
            .unwrap()
            .members
            .insert(member_id.clone(), membership);
        stub.clear_log();
        assert_eq!(
            refresh(&member, 24).await.unwrap().mainline().selected(),
            names.last()
        );
        assert_eq!(stub.request_lines(), vec![stream_listing(&id, 24)]);
        assert_eq!(
            managed(&member).status(&bundle(24)).unwrap().state,
            ManagedCloudStatusState::Ready
        );
        assert_eq!(
            managed(&member).repository_in_use(&bundle(24)).unwrap(),
            Some(id)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_a_gone_answer_reads_as_empty() {
        let stub = StubManagedCloud::start().await;
        let (store, user_id) = store(&stub, "octocat").await;
        let (id, names) = pushed(&stub, &store, 25, 2).await;
        let refused = |state: ManagedCloudStatusState, fault: Option<ListingFault>| {
            let store = store.clone();
            let stub = &stub;
            let id = id.clone();
            let parent = names.last().cloned();
            async move {
                stub.state.lock().unwrap().fail_next_listing = fault;
                stub.clear_log();
                let error = refresh(&store, 25).await.unwrap_err();
                assert_eq!(managed(&store).status(&bundle(25)).unwrap().state, state);
                // One request, and nothing asked of the service afterwards.
                assert_eq!(stub.request_lines(), vec![stream_listing(&id, 25)]);
                assert_eq!(
                    managed(&store).repository_in_use(&bundle(25)).unwrap(),
                    Some(id.clone())
                );
                // A Push stops at its first listing: nothing is created, nothing is written.
                stub.state.lock().unwrap().fail_next_listing = fault;
                stub.clear_log();
                let next = manifest(identity(25), 3, 300);
                let pushing = store.clone();
                let bytes = serde_json::to_vec(&next).unwrap();
                let (name, record) = record_of(&next, &bytes, parent.as_ref(), 3_000);
                let failed = blocking(move || {
                    let stream = RecordStream::project(&bundle(25)).unwrap();
                    write_record(&pushing, &stream, &name, &record)
                })
                .await;
                assert!(failed.is_err());
                assert_eq!(stub.request_lines(), vec![stream_listing(&id, 25)]);
                assert!(written_to(stub).is_empty());
                stub.state.lock().unwrap().fail_next_listing = None;
                error
            }
        };
        // 503.
        refused(
            ManagedCloudStatusState::Unavailable,
            Some(ListingFault::Unavailable),
        )
        .await;
        // A 404 that is not the service's.
        refused(
            ManagedCloudStatusState::Unavailable,
            Some(ListingFault::ForeignNotFound),
        )
        .await;
        // 403: the account is suspended.
        stub.state.lock().unwrap().suspended.insert(user_id.clone());
        let error = refused(ManagedCloudStatusState::Inaccessible, None).await;
        assert_eq!(
            error,
            "Mallard Cloud refused this account. It may be suspended."
        );
        stub.state.lock().unwrap().suspended.remove(&user_id);
        // The stream is what it was.
        assert_eq!(
            refresh(&store, 25).await.unwrap().mainline().selected(),
            names.last()
        );

        // An unreachable service: a port nothing listens on.
        let dead = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port())
        };
        let token = stub.state.lock().unwrap().add_session(&user_id);
        let client =
            ManagedCloudClient::new(&dead, Some(SecretValue::parse(&token).unwrap())).unwrap();
        let offline = ManagedObjectStore::new(client, Handle::current());
        offline.set_pin(&bundle(25), Some(&id)).unwrap();
        let listed = blocking({
            let offline = offline.clone();
            move || {
                let stream =
                    ObjectPrefix::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{}", bundle(25)))
                        .unwrap();
                offline.list_keys(&stream, None, 1_000)
            }
        })
        .await;
        assert!(listed.unwrap_err().contains("could not be reached"));
        assert_eq!(
            offline.status(&bundle(25)).unwrap().state,
            ManagedCloudStatusState::Unavailable
        );
        assert_eq!(offline.repository_in_use(&bundle(25)).unwrap(), Some(id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_create_refused_after_a_gone_answer_fails_the_push_and_the_next_listing_adopts() {
        let stub = StubManagedCloud::start().await;
        let (first, _) = store(&stub, "octocat").await;
        let (second, _) = {
            let user = stub
                .state
                .lock()
                .unwrap()
                .users
                .values()
                .next()
                .unwrap()
                .clone();
            let token = stub.state.lock().unwrap().add_session(&user.id);
            let client =
                ManagedCloudClient::new(&stub.endpoint, Some(SecretValue::parse(&token).unwrap()))
                    .unwrap();
            (ManagedObjectStore::new(client, Handle::current()), user.id)
        };
        let (old, _) = pushed(&stub, &first, 26, 1).await;
        second.set_pin(&bundle(26), Some(&old)).unwrap();
        stub.state.lock().unwrap().remove(&old);
        // Both computers list the gone repository; the first one pushes.
        let stream =
            ObjectPrefix::parse(format!("{REPOSITORY_COMMITS_PREFIX}/{}", bundle(26))).unwrap();
        let listed = blocking({
            let second = second.clone();
            let stream = stream.clone();
            move || second.list_keys(&stream, None, 1_000)
        })
        .await
        .unwrap();
        assert!(listed.keys.is_empty());
        assert!(refresh(&first, 26).await.unwrap().mainline().is_empty());
        let theirs = manifest(identity(26), 1, 300);
        let name = blocking({
            let first = first.clone();
            move || stored(push(&first, &theirs, None, 3_000))
        })
        .await;
        let new = repository_id(&stub, 26);
        // The second one's manifest write asks for a repository the account now has.
        stub.clear_log();
        let ours = manifest(identity(26), 1, 301);
        let error = blocking({
            let second = second.clone();
            let ours = ours.clone();
            move || second.put_immutable(&manifest_key(&ours), &serde_json::to_vec(&ours).unwrap())
        })
        .await
        .unwrap_err();
        assert_eq!(error, REMOTE_CHANGED);
        assert_eq!(
            stub.request_lines(),
            vec!["POST /v1/repositories".to_string()]
        );
        assert_eq!(second.repository_in_use(&bundle(26)).unwrap(), None);
        // Its next listing finds the repository and reads it.
        let listed = blocking({
            let second = second.clone();
            move || second.list_keys(&stream, None, 1_000)
        })
        .await
        .unwrap();
        assert_eq!(
            listed.keys.last().map(|key| key.as_str().to_owned()),
            Some(format!(
                "{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                bundle(26),
                name.file_name()
            ))
        );
        assert_eq!(second.repository_in_use(&bundle(26)).unwrap(), Some(new));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_discard_on_a_public_repository_reads_as_its_public_version() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let (id, names) = pushed(&stub, &store, 27, 2).await;
        // Version 2 is released, then the draft moves on by two pushes.
        stub.state.lock().unwrap().release(&id, None);
        refresh(&store, 27).await.unwrap();
        let draft = blocking({
            let store = store.clone();
            let released = names[1].clone();
            move || {
                let third = stored(push(
                    &store,
                    &manifest(identity(27), 3, 300),
                    Some(&released),
                    3_000,
                ));
                let fourth = stored(push(
                    &store,
                    &manifest(identity(27), 4, 400),
                    Some(&third),
                    4_000,
                ));
                vec![third, fourth]
            }
        })
        .await;
        let before = managed(&store).status(&bundle(27)).unwrap();
        assert_eq!(
            before.publication.as_ref().unwrap().state,
            PublicationState::UnpublishedDraft
        );
        let objects_before = stub.state.lock().unwrap().repositories[&id].objects.len();
        assert_eq!(stub.state.lock().unwrap().discard(&id), Ok(Some(2)));
        {
            let state = stub.state.lock().unwrap();
            let repository = &state.repositories[&id];
            // Two records, their claims, and their manifests were deleted.
            assert_eq!(repository.objects.len(), objects_before - 6);
            assert_eq!(
                repository
                    .keys_under(&format!("{REPOSITORY_PREFIX}/{}/_manifests", bundle(27)))
                    .len(),
                2
            );
            assert_eq!(
                repository
                    .keys_under(&format!("{REPOSITORY_CLAIMS_PREFIX}/{}", bundle(27)))
                    .len(),
                2
            );
            assert_eq!(
                repository.private_head.as_deref(),
                Some(names[1].file_name().as_str())
            );
            assert_eq!(
                repository
                    .publications
                    .iter()
                    .map(|publication| publication.state.as_str())
                    .collect::<Vec<_>>(),
                ["retired", "available", "removed", "removed"]
            );
        }
        // The listing selects the released push. Its removed child, which
        // this process saw, costs one direct read, once.
        stub.clear_log();
        assert_eq!(
            refresh(&store, 27).await.unwrap().mainline().selected(),
            Some(&names[1])
        );
        assert_eq!(
            stub.request_lines(),
            vec![
                stream_listing(&id, 27),
                format!(
                    "GET /v1/repositories/{id}/store/{REPOSITORY_COMMITS_PREFIX}/{}/{}",
                    bundle(27),
                    draft[0].file_name()
                ),
            ]
        );
        let status = managed(&store).status(&bundle(27)).unwrap();
        assert_eq!(status.state, ManagedCloudStatusState::Ready);
        assert_eq!(status.link.as_ref().unwrap().visibility, "public");
        assert_eq!(status.link.as_ref().unwrap().release, ReleaseState::Public);
        let publication = status.publication.unwrap();
        assert_eq!(publication.state, PublicationState::MatchesPublished);
        assert_eq!(publication.private_revision, Some(2));
        stub.clear_log();
        refresh(&store, 27).await.unwrap();
        assert_eq!(stub.request_lines(), vec![stream_listing(&id, 27)]);
        // The next push lands as the child of the released push.
        let next = blocking({
            let store = store.clone();
            let released = names[1].clone();
            move || {
                stored(push(
                    &store,
                    &manifest(identity(27), 3, 500),
                    Some(&released),
                    5_000,
                ))
            }
        })
        .await;
        assert_eq!(next.parent(), Some(names[1].token()));
        let status = managed(&store).status(&bundle(27)).unwrap();
        assert_eq!(status.link.unwrap().release, ReleaseState::SnapshotChanged);
        assert_eq!(
            status.publication.unwrap().state,
            PublicationState::UnpublishedDraft
        );
        // A draft that is the public version leaves nothing to discard, and
        // a repository made private is emptied like any private one.
        let mut state = stub.state.lock().unwrap();
        assert_eq!(state.discard(&id), Ok(Some(2)));
        assert_eq!(state.discard(&id), Err("There is no draft to discard."));
        state.make_private(&id);
        assert_eq!(state.discard(&id), Ok(None));
        assert!(state.repositories[&id].private_head.is_none());
        assert_eq!(state.discard(&id), Err("There is no draft to discard."));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_after_discard_does_not_renew_an_active_push_revision() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "fenced").await;
        let (id, names) = pushed(&stub, &store, 39, 1).await;
        let pending = manifest(identity(39), 2, 200);
        let pending_bytes = serde_json::to_vec(&pending).unwrap();
        blocking({
            let store = store.clone();
            let pending = pending.clone();
            let bytes = pending_bytes.clone();
            move || {
                store
                    .put_immutable(&manifest_key(&pending), &bytes)
                    .unwrap()
            }
        })
        .await;
        stub.state.lock().unwrap().discard(&id).unwrap();
        refresh(&store, 39).await.unwrap();
        let stream = RecordStream::project(&bundle(39)).unwrap();
        let claim = stream.claim_key(Some(names[0].token())).unwrap();
        let (_, record) = record_of(&pending, &pending_bytes, Some(&names[0]), 2_000);
        stub.clear_log();
        let failed = blocking({
            let store = store.clone();
            move || store.put_claim(&claim, &record).unwrap_err()
        })
        .await;
        assert!(failed.contains("revision_conflict"), "{failed}");
        assert_eq!(
            stub.requests()
                .iter()
                .filter(|request| request.method == "PUT")
                .count(),
            1
        );
        assert!(stub.state.lock().unwrap().repositories[&id]
            .keys_under(&format!("{REPOSITORY_CLAIMS_PREFIX}/{}", bundle(39)))
            .is_empty());
        // A new Push uses the revision the refresh observed.
        blocking({
            let store = store.clone();
            move || stored(push(&store, &manifest(identity(39), 1, 300), None, 3_000))
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_discard_is_one_request_of_the_owner_or_an_admin_and_the_store_deletes_nothing() {
        let stub = StubManagedCloud::start().await;
        let (owner, _) = store(&stub, "rika").await;
        let (admin, admin_id) = store(&stub, "tom").await;
        let (member, member_id) = store(&stub, "jeff").await;
        let (contributor, _) = store(&stub, "guest").await;
        let (id, names) = pushed(&stub, &owner, 28, 2).await;
        {
            let mut state = stub.state.lock().unwrap();
            state.add_member(&id, &admin_id, "admin");
            state.add_member(&id, &member_id, "member");
            state.release(&id, None);
        }
        refresh(&owner, 28).await.unwrap();
        let draft = blocking({
            let owner = owner.clone();
            let released = names[1].clone();
            move || {
                stored(push(
                    &owner,
                    &manifest(identity(28), 3, 300),
                    Some(&released),
                    3_000,
                ))
            }
        })
        .await;
        let revision = managed(&owner)
            .status(&bundle(28))
            .unwrap()
            .link
            .unwrap()
            .repository_revision;
        let client = |store: &ConfiguredObjectStore| managed(store).client().clone();

        // The store has no operation that deletes: a key and the whole
        // storage are both refused, without a request.
        stub.clear_log();
        blocking({
            let owner = owner.clone();
            let key = RecordStream::project(&bundle(28))
                .unwrap()
                .key(&draft)
                .unwrap();
            move || {
                assert!(owner.delete(&key, "any").is_err());
                assert!(owner.clear_managed_data().is_err());
            }
        })
        .await;
        assert!(stub.request_lines().is_empty());

        // A Member's and a contributor's discard are refused with the
        // service's sentence, and a stale revision is refused for the Owner.
        for refused in [&member, &contributor] {
            let error = client(refused)
                .discard_private_snapshot(&id, revision)
                .await
                .unwrap_err();
            assert_eq!(error.kind, ErrorKind::Permission);
            assert_eq!(error.code, Some("forbidden"));
            assert_eq!(
                error.message(),
                "Mallard Cloud refused this request. Only the Owner or an Admin can do this. (v1/repositories/{id}/private-snapshot/discard, HTTP 403, forbidden, request stub-request)"
            );
        }
        let stale = client(&owner)
            .discard_private_snapshot(&id, revision + 1)
            .await
            .unwrap_err();
        assert_eq!(stale.kind, ErrorKind::Conflict);
        assert_eq!(stale.code, Some("revision_conflict"));
        assert!(stale
            .message()
            .contains("The repository changed since you read it. Reload and retry."));
        assert_eq!(
            refresh(&owner, 28).await.unwrap().mainline().selected(),
            Some(&draft)
        );
        assert_eq!(
            stub.state.lock().unwrap().repositories[&id].revision,
            revision
        );

        // The Owner's: one request, naming the revision the status showed.
        stub.clear_log();
        let answer = client(&owner)
            .discard_private_snapshot(&id, revision)
            .await
            .unwrap();
        assert_eq!(
            stub.request_lines(),
            vec![format!(
                "POST /v1/repositories/{id}/private-snapshot/discard"
            )]
        );
        assert_eq!(answer.repository_id, id);
        assert_eq!(answer.state, "done");
        assert_eq!(answer.kept_revision, Some(2));
        assert_eq!(
            refresh(&owner, 28).await.unwrap().mainline().selected(),
            Some(&names[1])
        );
        // The draft is the public version: there is nothing to discard.
        let nothing = client(&owner)
            .discard_private_snapshot(&id, revision + 1)
            .await
            .unwrap_err();
        assert_eq!(nothing.kind, ErrorKind::Invalid);
        assert!(nothing.message().contains("There is no draft to discard."));

        // The service accepts an Admin's; the desktop offers the verb to the Owner alone.
        blocking({
            let owner = owner.clone();
            let released = names[1].clone();
            move || {
                stored(push(
                    &owner,
                    &manifest(identity(28), 3, 400),
                    Some(&released),
                    4_000,
                ))
            }
        })
        .await;
        let answer = client(&admin)
            .discard_private_snapshot(&id, revision + 1)
            .await
            .unwrap();
        assert_eq!(answer.kept_revision, Some(2));
        assert_eq!(
            refresh(&owner, 28).await.unwrap().mainline().selected(),
            Some(&names[1])
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_library_item_whose_repository_is_gone_lists_empty_and_its_push_creates_it_again() {
        let stub = StubManagedCloud::start().await;
        let (store, _) = store(&stub, "octocat").await;
        let stream_id = format!("library-{}", "5".repeat(32));
        let resource_id = format!("resource-{}", "6".repeat(64));
        let item = |revision: u64, parent: Option<&RecordName>| {
            let commit = mallard_domain::generated_named_id("commit").unwrap();
            let manifest = serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "repository_id": stream_id, "revision": revision, "commit_id": commit, "updated_at": 100,
                "captured_with": {"app": "mallard", "version": "test"},
                "published_by_origin": {"origin_host_token": "a".repeat(64), "origin_replica_token": "b".repeat(64)},
                "resources": {resource_id.clone(): {"resource": {"resource_id": resource_id, "provider": "codex", "resource_kind": "agent_skill",
                    "format": "agentskills.io.skill", "format_version": 1, "identity": {"root": "provider-home.skills", "effective_name": "my-skill", "install_directory": "my-skill"},
                    "descriptor": {}, "content_sha256": "0".repeat(64)}, "updated_at": 100}}
            }))
            .unwrap();
            let record = serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "repository_id": stream_id, "revision": revision, "commit_id": commit,
                "previous_commit_id": parent.map(|parent| parent.commit_id()),
                "previous_record_sha256": parent.map(|parent| parent.token().record_sha256()),
                "created_at_ms": revision * 1_000,
                "manifest_key": format!("_manifests/{revision}-{commit}.json"), "manifest_sha256": sha256_hex(&manifest), "manifest_size": manifest.len(),
                "published_by_origin": {"origin_host_token": "a".repeat(64), "origin_replica_token": "b".repeat(64)},
            }))
            .unwrap();
            let name = RecordName::new(
                revision,
                PublicationToken::new(commit.clone(), sha256_hex(&record)).unwrap(),
                parent.map(|parent| parent.token().clone()),
                revision * 1_000,
            )
            .unwrap();
            (
                format!("_manifests/{revision}-{commit}.json"),
                manifest,
                name,
                record,
            )
        };
        let push_item = |store: ConfiguredObjectStore,
                         (manifest_key, manifest, name, record): (
            String,
            Vec<u8>,
            RecordName,
            Vec<u8>,
        )| {
            let stream_id = stream_id.clone();
            blocking(move || {
                let key = crate::storage_core::library_object_key(&stream_id, &manifest_key)?;
                store.put_immutable(&key, &manifest)?;
                let stream = RecordStream::library(&stream_id)?;
                write_record(&store, &stream, &name, &record)?.stored()?;
                Ok::<_, String>(name)
            })
        };
        let listed = |store: ConfiguredObjectStore| {
            let stream_id = stream_id.clone();
            blocking(move || list_stream(&store, &RecordStream::library(&stream_id).unwrap()))
        };
        let first = push_item(store.clone(), item(1, None)).await.unwrap();
        let old = stub
            .state
            .lock()
            .unwrap()
            .repository_for_library(&stream_id)
            .unwrap()
            .id
            .clone();
        assert_eq!(
            listed(store.clone()).await.unwrap().mainline().selected(),
            Some(&first)
        );

        for finished in [false, true] {
            let gone = stub
                .state
                .lock()
                .unwrap()
                .repository_for_library(&stream_id)
                .unwrap()
                .id
                .clone();
            {
                let mut state = stub.state.lock().unwrap();
                state.remove(&gone);
                if finished {
                    state.finish_removal(&gone);
                }
            }
            // The store still holds the id it learned: this is the process
            // that pushed the item.
            stub.clear_log();
            assert!(listed(store.clone()).await.unwrap().mainline().is_empty());
            assert_eq!(
                stub.request_lines(),
                vec![
                    format!(
                        "GET /v1/repositories/{gone}/store?prefix={LIBRARY_PREFIX}/{stream_id}/_commits"
                    ),
                    format!("GET /v1/repositories/by-library/{stream_id}"),
                ]
            );
            // Its Push creates the repository again, at the same address.
            stub.clear_log();
            let again = push_item(store.clone(), item(1, None)).await.unwrap();
            let created = stub
                .state
                .lock()
                .unwrap()
                .repository_for_library(&stream_id)
                .unwrap()
                .clone();
            assert_ne!(created.id, gone);
            assert_eq!(created.name, "my-skill");
            assert_eq!(written_to(&stub), BTreeSet::from([created.id.clone()]));
            assert!(stub
                .request_lines()
                .contains(&"POST /v1/repositories".to_string()));
            // The item has a repository again, and nothing asks the old one.
            stub.clear_log();
            assert_eq!(
                listed(store.clone()).await.unwrap().mainline().selected(),
                Some(&again)
            );
            assert_eq!(
                stub.request_lines(),
                vec![format!(
                    "GET /v1/repositories/{}/store?prefix={LIBRARY_PREFIX}/{stream_id}/_commits",
                    created.id
                )]
            );
        }
        assert_ne!(
            stub.state
                .lock()
                .unwrap()
                .repository_for_library(&stream_id)
                .unwrap()
                .id,
            old
        );
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
