//! Immutable publication protocol for draft-linked Library resources.
//!
//! Each repository holds every resource of one coordinate as one set
//! manifest. The flat `.mallard/v1/catalog` namespace is deliberately
//! absent from this module, and so is repository enumeration: a repository is
//! always addressed by its derived id. There is no mutable head: a publish
//! ends by writing one immutable record, and the current set is selected
//! from the names of the repository's records.

use bytes::Bytes;
use mallard_domain::{Mainline, PublicationToken, RecordName, StorageConfig};
use mallard_infrastructure::storage_core::publication::{
    list_stream, read_record, write_record, RecordStream,
};
use mallard_infrastructure::storage_core::{
    library_object_key, ExpectedObject, ObjectStore, StageOutcome, TAG_FILE,
};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::domain::{
    library_manifest_relative_key, sha256_hex, CatalogCommitId, CatalogResourceId,
    CatalogResourceRepositoryId, LibraryHeadCoordinate, LibraryResourceCommit, LibraryResourceHead,
    LibraryResourceManifest, LibraryResourceTag, PortableCatalogOrigin, PublishedResourceEntry,
    CATALOG_SCHEMA, MAX_REVISION_PAYLOAD_BYTES,
};

const MAX_TAG_BYTES: u64 = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// The refusal an Apply ends with when the selected publication is no longer
/// the one its review was computed from. The Library does not rebase across
/// a remote change.
pub const CHANGED_AFTER_REVIEW: &str = "Library resource head changed after review";

/// The continuity identity of one storage: the provider key hashed with the
/// adapter's canonical coordinates. Never a credential or display name.
pub fn storage_identity_for_config(config: &StorageConfig) -> Result<String, String> {
    config.validate()?;
    let registry = StorageProviderRegistry::production();
    let adapter = registry.require(&config.provider, config.provider_config.version)?;
    adapter
        .validate_config(&config.provider_config)
        .map_err(|error| error.to_string())?;
    let coordinates = adapter.continuity_coordinates(&config.provider_config)?;
    Ok(super::domain::storage_connection_identity_sha256(
        config.provider.as_str(),
        &coordinates.iter().map(String::as_str).collect::<Vec<_>>(),
    ))
}

pub fn head_coordinate(
    head: &LibraryResourceHead,
    storage_identity_sha256: &str,
) -> LibraryHeadCoordinate {
    LibraryHeadCoordinate {
        repository_id: head.repository_id.clone(),
        revision: head.revision,
        commit_id: head.commit_id.clone(),
        manifest_sha256: head.manifest_sha256.clone(),
        manifest_size: head.manifest_size,
        storage_identity_sha256: storage_identity_sha256.to_string(),
    }
}

/// Whether the accepted publication is still part of the selected history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryContinuity {
    /// Nothing was accepted, or the accepted publication is the selected one
    /// or one of its ancestors.
    Continuous,
    /// The accepted publication is on no selected chain: the repository was
    /// cleared, with or without a Push since, or the accepted push was
    /// outranked after it was confirmed. The link's accepted state no longer
    /// describes this storage: it is read as absent and never reset, and the
    /// next Push or Pull replaces it.
    Replaced,
}

/// Continuity is decided by identity and ancestry, never by revision order:
/// two sibling publications share a revision. A repository that lists empty
/// holds no chain, so an accepted publication is not on it: the repository
/// was cleared, and one Push repopulates it. A changed storage identity and a
/// foreign repository id stay refusals.
pub fn verify_library_continuity(
    accepted: Option<&LibraryHeadCoordinate>,
    mainline: &Mainline,
    repository_id: &CatalogResourceRepositoryId,
    storage_identity_sha256: &str,
) -> Result<LibraryContinuity, String> {
    let Some(accepted) = accepted else {
        return Ok(LibraryContinuity::Continuous);
    };
    if &accepted.repository_id != repository_id {
        return Err("the storage link points at a different resource repository".to_string());
    }
    if accepted.storage_identity_sha256 != storage_identity_sha256 {
        return Err(
            "the storage connection identity changed after this resource was accepted".to_string(),
        );
    }
    Ok(
        if mainline.contains(accepted.revision, accepted.commit_id.as_str()) {
            LibraryContinuity::Continuous
        } else {
            LibraryContinuity::Replaced
        },
    )
}

/// The selected publication as one observation read it: its head view and
/// the digest of its record, which a publish names as its parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedLibraryHead {
    pub head: LibraryResourceHead,
    pub record_sha256: String,
}

impl ObservedLibraryHead {
    fn publication(&self) -> Result<PublicationToken, String> {
        PublicationToken::new(self.head.commit_id.as_str(), self.record_sha256.clone())
    }
}

pub struct PublishLibraryResourceRequest {
    pub repository_id: CatalogResourceRepositoryId,
    pub tag: LibraryResourceTag,
    pub observed: Option<ObservedLibraryHead>,
    /// The complete new published set. Publish replaces the manifest with
    /// exactly this set; callers build it as observed-set-plus-upserts.
    pub resources: BTreeMap<CatalogResourceId, PublishedResourceEntry>,
    pub payloads: BTreeMap<String, Bytes>,
    pub captured_with: super::domain::CatalogCapturedWith,
    pub published_by_origin: PortableCatalogOrigin,
    pub updated_at: u64,
}

#[derive(Debug)]
pub enum PublishedLibraryResource {
    /// The record was written and is the selected publication.
    Stored {
        head: LibraryResourceHead,
        manifest: LibraryResourceManifest,
    },
    /// The requested set already is the published one. Nothing was written.
    NoChange {
        head: LibraryResourceHead,
        manifest: LibraryResourceManifest,
    },
}

impl PublishedLibraryResource {
    pub fn head(&self) -> &LibraryResourceHead {
        match self {
            Self::Stored { head, .. } | Self::NoChange { head, .. } => head,
        }
    }

    pub fn manifest(&self) -> &LibraryResourceManifest {
        match self {
            Self::Stored { manifest, .. } | Self::NoChange { manifest, .. } => manifest,
        }
    }

    /// Whether this publish wrote a record, which is then the selected one.
    pub fn stored(&self) -> bool {
        matches!(self, Self::Stored { .. })
    }
}

pub struct LibraryResourceEngine<S> {
    store: S,
}

impl<S: ObjectStore> LibraryResourceEngine<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn read_tag(
        &self,
        repository_id: &CatalogResourceRepositoryId,
    ) -> Result<Option<(LibraryResourceTag, String)>, String> {
        let key = library_object_key(repository_id.as_str(), TAG_FILE)?;
        let Some(stored) = self.store.get_metadata(&key, MAX_TAG_BYTES)? else {
            return Ok(None);
        };
        let tag: LibraryResourceTag = serde_json::from_slice(&stored.bytes)
            .map_err(|error| format!("parse Library resource tag: {}", error))?;
        tag.validate()?;
        if &tag.repository_id != repository_id {
            return Err("Library resource tag is stored under another repository".to_string());
        }
        Ok(Some((tag, sha256_hex(&stored.bytes))))
    }

    /// Select, then read: list the repository's stream, select from the
    /// names, and read the selected record. Reading health never changes
    /// selection: a selected record that cannot be read or fails verification
    /// is an error, never a reason to try another one.
    pub fn observe(
        &self,
        repository_id: &CatalogResourceRepositoryId,
    ) -> Result<(Mainline, Option<ObservedLibraryHead>), String> {
        let stream = RecordStream::library(repository_id.as_str())?;
        let mainline = list_stream(&self.store, &stream)?.into_mainline();
        let Some(name) = mainline.selected() else {
            return Ok((mainline, None));
        };
        let bytes = read_record(&self.store, &stream, name)?;
        let head = parse_record(repository_id, name, &bytes)?.head();
        head.validate()?;
        let observed = ObservedLibraryHead {
            head,
            record_sha256: name.token().record_sha256().to_string(),
        };
        Ok((mainline, Some(observed)))
    }

    pub fn read_head(
        &self,
        repository_id: &CatalogResourceRepositoryId,
    ) -> Result<Option<ObservedLibraryHead>, String> {
        Ok(self.observe(repository_id)?.1)
    }

    pub fn read_manifest(
        &self,
        head: &LibraryResourceHead,
    ) -> Result<LibraryResourceManifest, String> {
        head.validate()?;
        let key = library_object_key(head.repository_id.as_str(), &head.manifest_key)?;
        let expected = ExpectedObject {
            size: head.manifest_size,
            sha256: head.manifest_sha256.clone(),
        };
        let body = match self.store.stage_verified(&key, &expected)? {
            StageOutcome::Missing => return Err("Library resource manifest is missing".to_string()),
            StageOutcome::Mismatch => {
                return Err("Library resource manifest failed integrity verification".to_string())
            }
            StageOutcome::Verified(body) => body,
        };
        let bytes = body.read_bounded(MAX_MANIFEST_BYTES, "Library resource manifest")?;
        let manifest: LibraryResourceManifest = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse Library resource manifest: {}", error))?;
        manifest.validate()?;
        if manifest.repository_id != head.repository_id
            || manifest.revision != head.revision
            || manifest.commit_id != head.commit_id
        {
            return Err("Library manifest coordinates do not match its head".to_string());
        }
        Ok(manifest)
    }

    pub fn inspect(
        &self,
        repository_id: &CatalogResourceRepositoryId,
    ) -> Result<Option<(ObservedLibraryHead, LibraryResourceManifest)>, String> {
        let Some(head) = self.read_head(repository_id)? else {
            return Ok(None);
        };
        let manifest = self.read_manifest(&head.head)?;
        Ok(Some((head, manifest)))
    }

    pub fn fetch_payload(
        &self,
        repository_id: &CatalogResourceRepositoryId,
        sha256: &str,
        size: u64,
    ) -> Result<Vec<u8>, String> {
        let key = library_object_key(repository_id.as_str(), &format!("_objects/{}", sha256))?;
        let expected = ExpectedObject {
            size,
            sha256: sha256.to_string(),
        };
        match self.store.stage_verified(&key, &expected)? {
            StageOutcome::Missing => Err(format!("Library payload '{}' is missing", key)),
            StageOutcome::Mismatch => Err(format!(
                "Library payload '{}' failed integrity verification",
                key
            )),
            StageOutcome::Verified(body) => body.read_bounded(size, "Library payload"),
        }
    }

    pub fn publish(
        &self,
        request: PublishLibraryResourceRequest,
    ) -> Result<PublishedLibraryResource, String> {
        request.tag.validate()?;
        if request.repository_id != request.tag.repository_id {
            return Err("Library publication tag does not match its repository".to_string());
        }
        if request.updated_at == 0 {
            return Err("Library publication requires a timestamp".to_string());
        }

        // Apply starts: list the stream and select. The Library does not
        // rebase: the selected publication must be the reviewed one.
        let current = self.read_head(&request.repository_id)?;
        if request.observed != current {
            return Err(CHANGED_AFTER_REVIEW.to_string());
        }
        let parent = current
            .as_ref()
            .map(ObservedLibraryHead::publication)
            .transpose()?;

        if let Some(current) = &current {
            let previous = self.read_manifest(&current.head)?;
            // A set whose per-resource content is unchanged is a no-op even
            // when entry timestamps differ; nothing is published for nothing.
            let previous_contents: BTreeMap<_, _> = previous
                .resources
                .iter()
                .map(|(id, entry)| (id.clone(), entry.resource.content_sha256.clone()))
                .collect();
            let requested_contents: BTreeMap<_, _> = request
                .resources
                .iter()
                .map(|(id, entry)| (id.clone(), entry.resource.content_sha256.clone()))
                .collect();
            if previous_contents == requested_contents {
                return Ok(PublishedLibraryResource::NoChange {
                    head: current.head.clone(),
                    manifest: previous,
                });
            }
        }

        let mut referenced: BTreeMap<String, u64> = BTreeMap::new();
        for entry in request.resources.values() {
            for (sha256, size) in entry.resource.referenced_payloads() {
                if let Some(existing) = referenced.insert(sha256.clone(), size) {
                    if existing != size {
                        return Err(
                            "Library publication references one payload at two sizes".to_string()
                        );
                    }
                }
            }
        }
        if request
            .payloads
            .keys()
            .any(|sha256| !referenced.contains_key(sha256))
        {
            return Err("Library publication includes an unreferenced payload".to_string());
        }

        let mut total_payload = 0_u64;
        let mut immutable = Vec::new();
        for (sha256, bytes) in &request.payloads {
            if sha256_hex(bytes) != *sha256 {
                return Err("Library payload bytes do not match their digest".to_string());
            }
            if referenced.get(sha256).copied() != Some(bytes.len() as u64) {
                return Err("Library payload bytes do not match their declared size".to_string());
            }
            total_payload = total_payload.saturating_add(bytes.len() as u64);
            immutable.push((
                library_object_key(
                    request.repository_id.as_str(),
                    &format!("_objects/{}", sha256),
                )?,
                bytes.clone(),
            ));
        }
        if total_payload > MAX_REVISION_PAYLOAD_BYTES {
            return Err("Library publication exceeds its payload limit".to_string());
        }
        self.store.put_many(immutable)?;
        for (sha256, size) in &referenced {
            if request.payloads.contains_key(sha256) {
                continue;
            }
            let key = library_object_key(
                request.repository_id.as_str(),
                &format!("_objects/{}", sha256),
            )?;
            let expected = ExpectedObject {
                size: *size,
                sha256: sha256.clone(),
            };
            match self.store.stage_verified(&key, &expected)? {
                StageOutcome::Verified(_) => {}
                StageOutcome::Missing => {
                    return Err(format!("Library payload '{}' is missing", key))
                }
                StageOutcome::Mismatch => {
                    return Err(format!(
                        "Library payload '{}' failed integrity verification",
                        key
                    ))
                }
            }
        }

        let revision = current
            .as_ref()
            .map(|observed| observed.head.revision.saturating_add(1))
            .unwrap_or(1);
        let commit_id = CatalogCommitId::generate()?;
        let manifest = LibraryResourceManifest {
            schema_version: CATALOG_SCHEMA,
            repository_id: request.repository_id.clone(),
            revision,
            commit_id: commit_id.clone(),
            updated_at: request.updated_at,
            captured_with: request.captured_with,
            published_by_origin: request.published_by_origin.clone(),
            resources: request.resources,
        };
        manifest.validate()?;
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| format!("serialize Library resource manifest: {}", error))?;
        if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err("Library resource manifest exceeds its size limit".to_string());
        }
        let manifest_key = library_manifest_relative_key(revision, &commit_id);
        self.store.put_immutable(
            &library_object_key(request.repository_id.as_str(), &manifest_key)?,
            &manifest_bytes,
        )?;
        // The tag freezes exactly the derivation identity; validate() above
        // already proved (provider, kind) matches the repository id, so every
        // writer writes identical bytes and a plain write is safe.
        let tag_bytes = serde_json::to_vec(&request.tag)
            .map_err(|error| format!("serialize Library resource tag: {}", error))?;
        self.store.put_immutable(
            &library_object_key(request.repository_id.as_str(), TAG_FILE)?,
            &tag_bytes,
        )?;

        // The record is built once, held in memory, and written last with
        // list-then-write. A change that lands between the apply-time listing
        // and this write stops it with nothing published.
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before the Unix epoch".to_string())?
            .as_millis();
        let commit = LibraryResourceCommit {
            schema_version: CATALOG_SCHEMA,
            repository_id: request.repository_id.clone(),
            revision,
            commit_id: commit_id.clone(),
            previous_commit_id: current
                .as_ref()
                .map(|observed| observed.head.commit_id.clone()),
            previous_record_sha256: current
                .as_ref()
                .map(|observed| observed.record_sha256.clone()),
            created_at_ms: u64::try_from(created_at_ms)
                .map_err(|_| "system clock is out of range".to_string())?,
            manifest_key,
            manifest_sha256: sha256_hex(&manifest_bytes),
            manifest_size: manifest_bytes.len() as u64,
            published_by_origin: request.published_by_origin,
        };
        commit.validate()?;
        let record_bytes = serde_json::to_vec(&commit)
            .map_err(|error| format!("serialize Library resource commit: {}", error))?;
        let name = RecordName::new(
            revision,
            PublicationToken::new(commit_id.as_str(), sha256_hex(&record_bytes))?,
            parent,
            commit.created_at_ms,
        )?;
        write_record(
            &self.store,
            &RecordStream::library(request.repository_id.as_str())?,
            &name,
            &record_bytes,
        )?;
        Ok(PublishedLibraryResource::Stored {
            head: commit.head(),
            manifest,
        })
    }
}

/// Parse verified record bytes. A name is self-certifying: the bytes already
/// hash to the digest in it, and the fields must equal its facts.
fn parse_record(
    repository_id: &CatalogResourceRepositoryId,
    name: &RecordName,
    bytes: &[u8],
) -> Result<LibraryResourceCommit, String> {
    let record: LibraryResourceCommit = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse Library resource commit: {}", error))?;
    record.validate()?;
    if &record.repository_id != repository_id {
        return Err("Library resource commit is stored under another repository".to_string());
    }
    let parent = match (&record.previous_commit_id, &record.previous_record_sha256) {
        (Some(commit_id), Some(record_sha256)) => Some(PublicationToken::new(
            commit_id.as_str(),
            record_sha256.clone(),
        )?),
        _ => None,
    };
    if record.revision != name.revision()
        || record.commit_id.as_str() != name.commit_id()
        || parent.as_ref() != name.parent()
        || record.created_at_ms != name.created_at_ms()
    {
        return Err("Library resource commit disagrees with its name".to_string());
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_infrastructure::storage_core::LocalObjectStore;
    use tempfile::tempdir;

    use crate::storage_catalog::domain::{
        CatalogCapturedWith, CatalogCoordinate, CatalogResourceEnvelope, ProviderKey,
    };
    use crate::storage_catalog::plugin_intent::{
        GitMarketplaceSource, PluginInstallIntent, PluginInstallIntentDescriptor,
        PluginInstallIntentIdentity, PluginResolutionPolicy, PortablePluginSource,
    };

    fn codex() -> ProviderKey {
        ProviderKey::parse("codex").unwrap()
    }

    fn plugin(plugin_id: &str, git_ref: &str) -> CatalogResourceEnvelope {
        let (_, namespace) = plugin_id.split_once('@').unwrap();
        PluginInstallIntent {
            identity: PluginInstallIntentIdentity {
                namespace: namespace.to_string(),
                plugin_id: plugin_id.to_string(),
            },
            descriptor: PluginInstallIntentDescriptor {
                source: PortablePluginSource::ProviderMarketplace {
                    namespace: namespace.to_string(),
                    git: Some(GitMarketplaceSource {
                        location: "https://github.com/example/review-market".to_string(),
                        git_ref: Some(git_ref.to_string()),
                    }),
                },
                resolution: PluginResolutionPolicy::ProviderResolved,
            },
        }
        .envelope(&codex())
        .unwrap()
    }

    fn plugin_coordinate() -> CatalogCoordinate {
        PluginInstallIntent::coordinate(&codex())
    }

    fn plugin_repository() -> CatalogResourceRepositoryId {
        CatalogResourceRepositoryId::for_coordinate(&plugin_coordinate())
    }

    fn tag(coordinate: &CatalogCoordinate) -> LibraryResourceTag {
        LibraryResourceTag::new(
            coordinate,
            CatalogResourceRepositoryId::for_coordinate(coordinate),
        )
    }

    fn set_of(
        resources: &[CatalogResourceEnvelope],
    ) -> BTreeMap<CatalogResourceId, PublishedResourceEntry> {
        resources
            .iter()
            .map(|resource| {
                (
                    resource.resource_id.clone(),
                    PublishedResourceEntry {
                        resource: resource.clone(),
                        updated_at: 1,
                    },
                )
            })
            .collect()
    }

    fn request(
        repository_id: &CatalogResourceRepositoryId,
        tag: LibraryResourceTag,
        observed: Option<ObservedLibraryHead>,
        resources: &[CatalogResourceEnvelope],
        updated_at: u64,
    ) -> PublishLibraryResourceRequest {
        PublishLibraryResourceRequest {
            repository_id: repository_id.clone(),
            tag,
            observed,
            resources: set_of(resources),
            payloads: BTreeMap::new(),
            captured_with: CatalogCapturedWith {
                app: "mallard".to_string(),
                version: "test".to_string(),
            },
            published_by_origin: PortableCatalogOrigin {
                origin_host_token: "a".repeat(64),
                origin_replica_token: "b".repeat(64),
            },
            updated_at,
        }
    }

    fn publish<S: ObjectStore>(
        engine: &LibraryResourceEngine<S>,
        observed: Option<ObservedLibraryHead>,
        resources: &[CatalogResourceEnvelope],
        updated_at: u64,
    ) -> Result<PublishedLibraryResource, String> {
        engine.publish(request(
            &plugin_repository(),
            tag(&plugin_coordinate()),
            observed,
            resources,
            updated_at,
        ))
    }

    #[test]
    fn dropbox_two_users_publish_read_and_reject_stale_library_writes() {
        let server =
            mallard_infrastructure::storage_core::dropbox_store::mock::MockDropboxServer::new();
        let second = server.second_user();
        let first = LibraryResourceEngine::new(server.store);
        let second = LibraryResourceEngine::new(second);
        let id = plugin_repository();
        let resource = plugin("review@review-market", "main");
        first
            .publish(request(
                &id,
                tag(&plugin_coordinate()),
                None,
                std::slice::from_ref(&resource),
                1,
            ))
            .unwrap();
        let (observed, manifest) = second.inspect(&id).unwrap().unwrap();
        assert_eq!(
            manifest.resource_map().get(&resource.resource_id),
            Some(&resource)
        );
        let changed = plugin("review@review-market", "next");
        second
            .publish(request(
                &id,
                tag(&plugin_coordinate()),
                Some(observed.clone()),
                std::slice::from_ref(&changed),
                2,
            ))
            .unwrap();
        let error = first
            .publish(request(
                &id,
                tag(&plugin_coordinate()),
                Some(observed),
                &[resource],
                3,
            ))
            .unwrap_err();
        assert_eq!(error, CHANGED_AFTER_REVIEW);
        assert_eq!(
            first
                .inspect(&id)
                .unwrap()
                .unwrap()
                .1
                .resource_map()
                .get(&changed.resource_id),
            Some(&changed)
        );
    }

    #[test]
    fn storage_identity_rejects_a_provider_invalid_configuration() {
        let temporary = tempdir().unwrap();
        let mut config = mallard_infrastructure::storage_providers::local::local_storage_config(
            mallard_domain::StorageId::parse("storage-invalid").unwrap(),
            "Invalid",
            &temporary.path().join("storage"),
        );
        config
            .provider_config
            .secrets
            .insert(
                mallard_domain::SecretKey::parse("unexpected").unwrap(),
                mallard_domain::SecretValue::parse("secret-canary").unwrap(),
            )
            .unwrap();
        assert!(config.validate().is_ok());

        let error = storage_identity_for_config(&config).unwrap_err();

        assert_eq!(error, "A local folder storage has no secrets.");
        assert!(!error.contains("secret-canary"));
    }

    #[test]
    fn one_coordinate_repository_grows_its_set_and_rejects_a_stale_writer() {
        let temporary = tempdir().unwrap();
        let store = LocalObjectStore::open(temporary.path()).unwrap();
        let engine = LibraryResourceEngine::new(store);
        let repository_id = plugin_repository();
        let first_resource = plugin("review@review-market", "main");
        let first = publish(&engine, None, std::slice::from_ref(&first_resource), 1).unwrap();
        assert!(matches!(first, PublishedLibraryResource::Stored { .. }));
        assert_eq!(first.head().revision, 1);
        assert_eq!(
            engine
                .inspect(&repository_id)
                .unwrap()
                .unwrap()
                .1
                .resource_map(),
            BTreeMap::from([(first_resource.resource_id.clone(), first_resource.clone())])
        );
        let (stored_tag, _) = engine.read_tag(&repository_id).unwrap().unwrap();
        assert_eq!(stored_tag.coordinate(), plugin_coordinate());

        // The set grows: a second resource joins the first in one manifest.
        let stale = engine.read_head(&repository_id).unwrap().unwrap();
        let second_resource = plugin("tools@review-market", "main");
        let second = publish(
            &engine,
            Some(stale.clone()),
            &[first_resource.clone(), second_resource.clone()],
            2,
        )
        .unwrap();
        assert_eq!(second.head().revision, 2);
        assert_eq!(second.manifest().resources.len(), 2);

        // A writer holding the superseded publication is refused.
        let error = publish(
            &engine,
            Some(stale),
            &[
                first_resource,
                second_resource,
                plugin("late@review-market", "next"),
            ],
            3,
        )
        .unwrap_err();
        assert_eq!(error, CHANGED_AFTER_REVIEW);
        assert_eq!(
            engine
                .read_head(&repository_id)
                .unwrap()
                .unwrap()
                .head
                .revision,
            2
        );

        // The record is the publication: no head object is ever written, the
        // tag holds no timestamp, and records live inside the repository.
        let root = temporary
            .path()
            .join(".mallard/v1/library")
            .join(repository_id.as_str());
        assert!(!root.join("_head.json").exists());
        assert_eq!(std::fs::read_dir(root.join("_commits")).unwrap().count(), 2);
        assert_eq!(
            std::fs::read(root.join("_tag.json")).unwrap(),
            serde_json::to_vec(&tag(&plugin_coordinate())).unwrap()
        );
    }

    #[test]
    fn an_identical_set_publishes_nothing() {
        let temporary = tempdir().unwrap();
        let store = LocalObjectStore::open(temporary.path()).unwrap();
        let engine = LibraryResourceEngine::new(store);
        let resource = plugin("review@review-market", "main");
        publish(&engine, None, std::slice::from_ref(&resource), 1).unwrap();

        let observed = engine.read_head(&plugin_repository()).unwrap();
        let repeat = publish(&engine, observed, &[resource], 2).unwrap();
        assert!(!repeat.stored());
        assert_eq!(repeat.head().revision, 1);
    }

    #[test]
    fn a_change_between_the_apply_listing_and_the_record_write_is_a_changed_remote() {
        use mallard_infrastructure::storage_core::plain_store::PlainStore;
        use mallard_infrastructure::storage_core::publication::REMOTE_CHANGED;
        let shared = PlainStore::default();
        let engine = LibraryResourceEngine::new(shared.clone());
        let repository_id = plugin_repository();
        let base = plugin("review@review-market", "main");
        publish(&engine, None, std::slice::from_ref(&base), 1).unwrap();
        let reviewed = engine.read_head(&repository_id).unwrap();

        // Another computer's push lands after this Apply listed and selected
        // the reviewed publication, and before it lists for its record.
        let rival = LibraryResourceEngine::new(shared.clone());
        let rival_reviewed = reviewed.clone();
        let rival_base = base.clone();
        let intruder = std::sync::Mutex::new(Some(move || {
            publish(
                &rival,
                rival_reviewed,
                &[rival_base, plugin("tools@review-market", "main")],
                2,
            )
            .unwrap();
        }));
        let hooked = HookedStore {
            inner: shared.clone(),
            before_manifest: intruder,
        };
        let error = publish(
            &LibraryResourceEngine::new(hooked),
            reviewed,
            &[base, plugin("late@review-market", "next")],
            3,
        )
        .unwrap_err();
        assert_eq!(error, REMOTE_CHANGED);
        // Nothing of the stopped Apply was published.
        let (_, manifest) = engine.inspect(&repository_id).unwrap().unwrap();
        assert_eq!(manifest.revision, 2);
        assert_eq!(manifest.resources.len(), 2);
    }

    /// Runs one action right before the manifest write: after the apply-time
    /// listing, before the listing that precedes the record.
    struct HookedStore<F: FnOnce() + Send> {
        inner: mallard_infrastructure::storage_core::plain_store::PlainStore,
        before_manifest: std::sync::Mutex<Option<F>>,
    }

    impl<F: FnOnce() + Send> ObjectStore for HookedStore<F> {
        fn get_metadata(
            &self,
            key: &mallard_infrastructure::storage_core::ObjectKey,
            max_bytes: u64,
        ) -> Result<Option<mallard_infrastructure::storage_core::StoredMetadata>, String> {
            self.inner.get_metadata(key, max_bytes)
        }

        fn stage_verified(
            &self,
            key: &mallard_infrastructure::storage_core::ObjectKey,
            expected: &ExpectedObject,
        ) -> Result<StageOutcome, String> {
            self.inner.stage_verified(key, expected)
        }

        fn put_immutable(
            &self,
            key: &mallard_infrastructure::storage_core::ObjectKey,
            bytes: &[u8],
        ) -> Result<mallard_infrastructure::storage_core::ImmutablePutOutcome, String> {
            if key.as_str().contains("/_manifests/") {
                if let Some(action) = self.before_manifest.lock().unwrap().take() {
                    action();
                }
            }
            self.inner.put_immutable(key, bytes)
        }

        fn put_record(
            &self,
            key: &mallard_infrastructure::storage_core::ObjectKey,
            bytes: &[u8],
        ) -> Result<mallard_infrastructure::storage_core::ImmutablePutOutcome, String> {
            self.inner.put_record(key, bytes)
        }

        fn list(
            &self,
            prefix: &mallard_infrastructure::storage_core::ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<mallard_infrastructure::storage_core::StoreListPage, String> {
            self.inner.list(prefix, cursor, limit)
        }

        fn list_keys(
            &self,
            prefix: &mallard_infrastructure::storage_core::ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<mallard_infrastructure::storage_core::StoreKeyPage, String> {
            self.inner.list_keys(prefix, cursor, limit)
        }
    }

    #[test]
    fn continuity_is_ancestry_and_a_push_that_loses_the_fork_window_fails() {
        use mallard_infrastructure::storage_core::plain_store::PlainStore;
        use mallard_infrastructure::storage_core::publication::REMOTE_CHANGED;
        let shared = PlainStore::default();
        let engine = LibraryResourceEngine::new(shared.clone());
        let repository_id = plugin_repository();
        let identity = "a".repeat(64);
        let base = plugin("review@review-market", "main");
        let first = publish(&engine, None, std::slice::from_ref(&base), 1).unwrap();
        let reviewed = engine.read_head(&repository_id).unwrap();
        let accepted_first = head_coordinate(first.head(), &identity);

        // Two computers push on the same reviewed publication inside the
        // fork window: the earlier record stalls before its one request, and
        // lands between the later writer's listing and its write.
        let (stalled_tx, stalled_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel::<()>();
        shared.before_next_record_request(move || {
            stalled_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        });
        let earlier = std::thread::spawn({
            let engine = LibraryResourceEngine::new(shared.clone());
            let reviewed = reviewed.clone();
            let resources = [base.clone(), plugin("tools@review-market", "main")];
            move || publish(&engine, reviewed, &resources, 2)
        });
        stalled_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        shared.before_next_record_request({
            let store = shared.clone();
            move || {
                resume_tx.send(()).unwrap();
                while store
                    .keys()
                    .iter()
                    .filter(|key| key.as_str().contains("/_commits/"))
                    .count()
                    < 2
                {
                    std::thread::yield_now();
                }
            }
        });
        let later = publish(
            &engine,
            reviewed,
            &[base, plugin("late@review-market", "next")],
            3,
        )
        .unwrap_err();
        let earlier = earlier.join().unwrap().unwrap();
        // The push whose record ranks second failed as a whole, so nothing
        // of it is accepted. Its record is kept in the repository.
        assert_eq!(later, REMOTE_CHANGED);
        assert!(earlier.stored());

        let (mainline, selected) = engine.observe(&repository_id).unwrap();
        assert_eq!(selected.unwrap().head, *earlier.head());
        let kept = shared
            .keys()
            .iter()
            .filter(|key| key.as_str().contains("/_commits/"))
            .filter_map(|key| RecordName::parse(key.as_str().rsplit('/').next()?))
            .find(|name| {
                name.commit_id() != first.head().commit_id.as_str()
                    && name.commit_id() != earlier.head().commit_id.as_str()
            })
            .expect("the record that was not selected is kept");
        let stream = RecordStream::library(repository_id.as_str()).unwrap();
        let kept_head = parse_record(
            &repository_id,
            &kept,
            &read_record(&shared, &stream, &kept).unwrap(),
        )
        .unwrap()
        .head();
        let verdict = |accepted: &LibraryHeadCoordinate| {
            verify_library_continuity(Some(accepted), &mainline, &repository_id, &identity)
        };
        // The first publication is an ancestor, and the winner is selected.
        assert_eq!(verdict(&accepted_first), Ok(LibraryContinuity::Continuous));
        assert_eq!(
            verdict(&head_coordinate(earlier.head(), &identity)),
            Ok(LibraryContinuity::Continuous)
        );
        // A record that is not selected shares the winner's revision and is
        // on no selected chain: revision order would have called it current.
        // A link can hold one only when it was outranked after its push was
        // confirmed, and it then reads `Replaced`.
        assert_eq!(earlier.head().revision, kept_head.revision);
        assert_eq!(
            verdict(&head_coordinate(&kept_head, &identity)),
            Ok(LibraryContinuity::Replaced)
        );
        assert_eq!(
            verify_library_continuity(None, &mainline, &repository_id, &identity),
            Ok(LibraryContinuity::Continuous)
        );

        // B1: the repository was cleared and nobody pushed since. It holds
        // no chain, so the accepted publication is not on it. That is a
        // replaced history like any other, never a refusal: the page loads,
        // and one Push repopulates the repository.
        assert_eq!(
            verify_library_continuity(
                Some(&accepted_first),
                &Mainline::default(),
                &repository_id,
                &identity
            ),
            Ok(LibraryContinuity::Replaced)
        );
        assert_eq!(
            verify_library_continuity(None, &Mainline::default(), &repository_id, &identity),
            Ok(LibraryContinuity::Continuous)
        );

        // B3: the refusals stay, on a cleared repository too: another storage
        // identity, and a foreign repository.
        for chain in [&mainline, &Mainline::default()] {
            assert!(verify_library_continuity(
                Some(&accepted_first),
                chain,
                &repository_id,
                &"b".repeat(64)
            )
            .unwrap_err()
            .contains("identity changed"));
        }
        let skills = CatalogResourceRepositoryId::for_coordinate(
            &CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap(),
        );
        assert!(
            verify_library_continuity(Some(&accepted_first), &mainline, &skills, &identity)
                .unwrap_err()
                .contains("different resource repository")
        );
    }

    #[test]
    fn a_repository_rejects_foreign_coordinates() {
        let temporary = tempdir().unwrap();
        let store = LocalObjectStore::open(temporary.path()).unwrap();
        let engine = LibraryResourceEngine::new(store);
        let skills =
            CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap();
        let skills_repository = CatalogResourceRepositoryId::for_coordinate(&skills);
        // A tag pinned to the plugin repository cannot publish under the
        // skills repository id: the derivation check fails closed.
        let error = engine
            .publish(request(
                &skills_repository,
                tag(&plugin_coordinate()),
                None,
                &[plugin("review@review-market", "main")],
                1,
            ))
            .unwrap_err();
        assert!(
            error.contains("tag does not match its repository"),
            "{error}"
        );

        // A plugin entry cannot enter the skills repository even under the
        // skills repository's own valid tag: the manifest's homogeneity
        // validation refuses the set.
        let error = engine
            .publish(request(
                &skills_repository,
                tag(&skills),
                None,
                &[plugin("review@review-market", "main")],
                1,
            ))
            .unwrap_err();
        assert!(
            error.contains("does not belong to this repository"),
            "{error}"
        );
    }
}
