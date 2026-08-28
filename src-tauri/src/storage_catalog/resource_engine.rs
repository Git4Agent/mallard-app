//! Immutable/CAS storage protocol for draft-linked Library resources.
//!
//! Each repository holds every resource of one (provider, kind) pair as one
//! set manifest. The flat `.mallard/v1/catalog` namespace is deliberately
//! absent from this module, and so is bucket listing: a repository is always
//! addressed by its derived id and read as one head plus one manifest.

use bytes::Bytes;
use mallard_domain::{StorageConfig, StorageKind};
use mallard_infrastructure::storage_core::{
    ensure_storage_marker, library_object_key, CasExpectation, CasOutcome, ExpectedObject,
    ImmutablePutOutcome, ObjectStore, StageOutcome, HEAD_FILE, TAG_FILE,
};
use std::collections::BTreeMap;

use super::domain::{
    library_commit_relative_key, library_manifest_relative_key, sha256_hex, CatalogCommitId,
    CatalogResource, CatalogResourceId, CatalogResourceRepositoryId, LibraryHeadCoordinate,
    LibraryResourceCommit, LibraryResourceHead, LibraryResourceManifest, LibraryResourceTag,
    PortableCatalogOrigin, PublishedResourceEntry, CATALOG_SCHEMA, MAX_REVISION_PAYLOAD_BYTES,
};

const MAX_TAG_BYTES: u64 = 64 * 1024;
const MAX_HEAD_BYTES: u64 = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

fn referenced_payloads(resource: &CatalogResource) -> BTreeMap<String, u64> {
    match resource {
        CatalogResource::AgentSkill(skill) => skill
            .files
            .values()
            .map(|entry| (entry.sha256.clone(), entry.size))
            .collect(),
        CatalogResource::CodexPetV1(pet) | CatalogResource::CodexPetV2(pet) => BTreeMap::from([(
            pet.descriptor.atlas.sha256.clone(),
            pet.descriptor.atlas.size,
        )]),
        CatalogResource::PluginInstallIntent(_) => BTreeMap::new(),
    }
}

pub fn storage_identity_for_config(config: &StorageConfig) -> String {
    match config.kind {
        StorageKind::Local => {
            super::domain::storage_connection_identity_sha256("local", &[config.local_dir.as_str()])
        }
        StorageKind::S3 => super::domain::storage_connection_identity_sha256(
            "s3",
            &[
                config.bucket.as_str(),
                config.account_id.as_str(),
                config.s3_endpoint.as_str(),
                config.region.as_str(),
            ],
        ),
    }
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

pub fn verify_library_continuity(
    accepted: Option<&LibraryHeadCoordinate>,
    observed: Option<&LibraryResourceHead>,
    repository_id: &CatalogResourceRepositoryId,
    storage_identity_sha256: &str,
) -> Result<(), String> {
    let Some(accepted) = accepted else {
        return Ok(());
    };
    if &accepted.repository_id != repository_id {
        return Err("the storage link points at a different resource repository".to_string());
    }
    if accepted.storage_identity_sha256 != storage_identity_sha256 {
        return Err(
            "the storage connection identity changed after this resource was accepted".to_string(),
        );
    }
    let Some(observed) = observed else {
        return Err("the accepted resource repository disappeared from storage".to_string());
    };
    if &observed.repository_id != repository_id {
        return Err("the resource repository changed identity".to_string());
    }
    if observed.revision < accepted.revision {
        return Err("the resource repository history was rewound".to_string());
    }
    if observed.revision == accepted.revision
        && (observed.commit_id != accepted.commit_id
            || observed.manifest_sha256 != accepted.manifest_sha256
            || observed.manifest_size != accepted.manifest_size)
    {
        return Err("the accepted resource repository revision was replaced".to_string());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ObservedLibraryHead {
    pub head: LibraryResourceHead,
    pub etag: String,
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
pub struct PublishedLibraryResource {
    pub head: LibraryResourceHead,
    pub manifest: LibraryResourceManifest,
    pub published: bool,
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

    pub fn read_head(
        &self,
        repository_id: &CatalogResourceRepositoryId,
    ) -> Result<Option<ObservedLibraryHead>, String> {
        let key = library_object_key(repository_id.as_str(), HEAD_FILE)?;
        let Some(stored) = self.store.get_metadata(&key, MAX_HEAD_BYTES)? else {
            return Ok(None);
        };
        let head: LibraryResourceHead = serde_json::from_slice(&stored.bytes)
            .map_err(|error| format!("parse Library resource head: {}", error))?;
        head.validate()?;
        if &head.repository_id != repository_id {
            return Err("Library resource head is stored under another repository".to_string());
        }
        Ok(Some(ObservedLibraryHead {
            head,
            etag: stored.etag,
        }))
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
        ensure_storage_marker(&self.store)?;
        request.tag.validate()?;
        if request.repository_id != request.tag.repository_id {
            return Err("Library publication tag does not match its repository".to_string());
        }
        if request.updated_at == 0 {
            return Err("Library publication requires a timestamp".to_string());
        }

        let current = self.read_head(&request.repository_id)?;
        match (&request.observed, &current) {
            (None, None) => {}
            (Some(expected), Some(current))
                if expected.etag == current.etag && expected.head == current.head => {}
            _ => return Err("Library resource head changed after review".to_string()),
        }

        // The tag freezes exactly the derivation identity; validate() above
        // already proved (provider, kind) matches the repository id, so any
        // existing tag under the same repository can only disagree on its
        // creation timestamp.
        let existing_tag = self.read_tag(&request.repository_id)?;

        if let Some(current) = &current {
            let previous = self.read_manifest(&current.head)?;
            // A set whose per-resource content is unchanged is a no-op even
            // when entry timestamps differ; the head never moves for nothing.
            let previous_contents: BTreeMap<_, _> = previous
                .resources
                .iter()
                .map(|(id, entry)| (id.clone(), entry.resource.content_sha256().to_string()))
                .collect();
            let requested_contents: BTreeMap<_, _> = request
                .resources
                .iter()
                .map(|(id, entry)| (id.clone(), entry.resource.content_sha256().to_string()))
                .collect();
            if previous_contents == requested_contents {
                return Ok(PublishedLibraryResource {
                    head: current.head.clone(),
                    manifest: previous,
                    published: false,
                });
            }
        }

        let mut referenced: BTreeMap<String, u64> = BTreeMap::new();
        for entry in request.resources.values() {
            for (sha256, size) in referenced_payloads(&entry.resource) {
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
        let manifest_sha256 = sha256_hex(&manifest_bytes);
        let manifest_size = manifest_bytes.len() as u64;
        let manifest_key = library_manifest_relative_key(revision, &commit_id);
        let commit_key = library_commit_relative_key(revision, &commit_id);
        let commit = LibraryResourceCommit {
            schema_version: CATALOG_SCHEMA,
            repository_id: request.repository_id.clone(),
            revision,
            commit_id: commit_id.clone(),
            manifest_key: manifest_key.clone(),
            manifest_sha256: manifest_sha256.clone(),
            created_at: request.updated_at,
            previous_commit_id: current
                .as_ref()
                .map(|observed| observed.head.commit_id.clone()),
            published_by_origin: request.published_by_origin,
        };
        commit.validate()?;
        let commit_bytes = serde_json::to_vec(&commit)
            .map_err(|error| format!("serialize Library resource commit: {}", error))?;
        for (key, bytes) in [
            (
                library_object_key(request.repository_id.as_str(), &manifest_key)?,
                manifest_bytes,
            ),
            (
                library_object_key(request.repository_id.as_str(), &commit_key)?,
                commit_bytes,
            ),
        ] {
            match self.store.put_immutable(&key, &bytes)? {
                ImmutablePutOutcome::Written | ImmutablePutOutcome::AlreadyPresent => {}
            }
        }
        if existing_tag.is_none() {
            let tag_bytes = serde_json::to_vec(&request.tag)
                .map_err(|error| format!("serialize Library resource tag: {}", error))?;
            self.store.put_immutable(
                &library_object_key(request.repository_id.as_str(), TAG_FILE)?,
                &tag_bytes,
            )?;
        }

        let head = LibraryResourceHead {
            schema_version: CATALOG_SCHEMA,
            repository_id: request.repository_id.clone(),
            revision,
            commit_id,
            manifest_key,
            manifest_sha256,
            manifest_size,
            updated_at: request.updated_at,
        };
        head.validate()?;
        let head_bytes = serde_json::to_vec(&head)
            .map_err(|error| format!("serialize Library resource head: {}", error))?;
        let expectation = current
            .as_ref()
            .map(|observed| CasExpectation::Match(observed.etag.clone()))
            .unwrap_or(CasExpectation::Absent);
        let head_key = library_object_key(request.repository_id.as_str(), HEAD_FILE)?;
        match self
            .store
            .compare_and_swap(&head_key, &expectation, &head_bytes)?
        {
            CasOutcome::Written { .. } => {}
            CasOutcome::Conflict { .. } => {
                let resolved = self
                    .read_head(&request.repository_id)?
                    .ok_or("Library head CAS conflicted and the head is now absent")?;
                if resolved.head != head {
                    return Err("Library resource head changed during publication".to_string());
                }
            }
        }
        Ok(PublishedLibraryResource {
            head,
            manifest,
            published: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_infrastructure::storage_core::LocalObjectStore;
    use tempfile::tempdir;

    use crate::storage_catalog::domain::{
        CatalogCapturedWith, CatalogResourceKindTag, GitMarketplaceSource,
        PluginInstallIntentDescriptor, PluginInstallIntentResource, PluginResolutionPolicy,
        PortablePluginSource, ProviderKey,
    };

    fn plugin(plugin_id: &str, git_ref: &str) -> CatalogResource {
        let descriptor = PluginInstallIntentDescriptor {
            provider: ProviderKey::Codex,
            provider_native_namespace: "review-market".to_string(),
            plugin_id: plugin_id.to_string(),
            source: PortablePluginSource::ProviderMarketplace {
                namespace: "review-market".to_string(),
                git: Some(GitMarketplaceSource {
                    location: "https://github.com/example/review-market".to_string(),
                    git_ref: Some(git_ref.to_string()),
                }),
            },
            resolution: PluginResolutionPolicy::ProviderResolved,
        };
        let mut resource = PluginInstallIntentResource {
            descriptor,
            content_sha256: "0".repeat(64),
        };
        resource.content_sha256 = resource.compute_content_sha256();
        CatalogResource::PluginInstallIntent(resource)
    }

    fn plugin_repository() -> CatalogResourceRepositoryId {
        CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::PluginInstallIntent,
        )
    }

    fn tag(repository_id: &CatalogResourceRepositoryId) -> LibraryResourceTag {
        LibraryResourceTag {
            schema_version: CATALOG_SCHEMA,
            repository_id: repository_id.clone(),
            provider: ProviderKey::Codex,
            resource_kind: CatalogResourceKindTag::PluginInstallIntent,
            created_at: 1,
        }
    }

    fn set_of(
        resources: &[CatalogResource],
    ) -> BTreeMap<CatalogResourceId, PublishedResourceEntry> {
        resources
            .iter()
            .map(|resource| {
                (
                    resource.resource_id(),
                    PublishedResourceEntry {
                        resource: resource.clone(),
                        updated_at: 1,
                    },
                )
            })
            .collect()
    }

    fn publish(
        engine: &LibraryResourceEngine<LocalObjectStore>,
        repository_id: &CatalogResourceRepositoryId,
        observed: Option<ObservedLibraryHead>,
        resources: &[CatalogResource],
        updated_at: u64,
    ) -> Result<PublishedLibraryResource, String> {
        engine.publish(PublishLibraryResourceRequest {
            repository_id: repository_id.clone(),
            tag: tag(repository_id),
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
        })
    }

    #[test]
    fn one_kind_repository_grows_its_set_and_rejects_a_stale_writer() {
        let temporary = tempdir().unwrap();
        let store = LocalObjectStore::open(temporary.path()).unwrap();
        let engine = LibraryResourceEngine::new(store);
        let repository_id = plugin_repository();
        let first_resource = plugin("review@review-market", "main");
        let first = publish(
            &engine,
            &repository_id,
            None,
            std::slice::from_ref(&first_resource),
            1,
        )
        .unwrap();
        assert_eq!(first.head.revision, 1);
        assert_eq!(
            engine
                .inspect(&repository_id)
                .unwrap()
                .unwrap()
                .1
                .resource_map(),
            BTreeMap::from([(first_resource.resource_id(), first_resource.clone())])
        );

        // The set grows: a second resource joins the first in one manifest.
        let stale = engine.read_head(&repository_id).unwrap().unwrap();
        let second_resource = plugin("tools@review-market", "main");
        let second = publish(
            &engine,
            &repository_id,
            Some(stale.clone()),
            &[first_resource.clone(), second_resource.clone()],
            2,
        )
        .unwrap();
        assert_eq!(second.head.revision, 2);
        assert_eq!(second.manifest.resources.len(), 2);

        // A writer holding the superseded head is refused.
        let error = publish(
            &engine,
            &repository_id,
            Some(stale),
            &[
                first_resource,
                second_resource,
                plugin("late@review-market", "next"),
            ],
            3,
        )
        .unwrap_err();
        assert!(error.contains("head changed after review"), "{error}");
        assert_eq!(
            engine
                .read_head(&repository_id)
                .unwrap()
                .unwrap()
                .head
                .revision,
            2
        );
    }

    #[test]
    fn an_identical_set_publishes_nothing() {
        let temporary = tempdir().unwrap();
        let store = LocalObjectStore::open(temporary.path()).unwrap();
        let engine = LibraryResourceEngine::new(store);
        let repository_id = plugin_repository();
        let resource = plugin("review@review-market", "main");
        publish(
            &engine,
            &repository_id,
            None,
            std::slice::from_ref(&resource),
            1,
        )
        .unwrap();

        let observed = engine.read_head(&repository_id).unwrap();
        let repeat = publish(&engine, &repository_id, observed, &[resource], 2).unwrap();
        assert!(!repeat.published);
        assert_eq!(repeat.head.revision, 1);
    }

    #[test]
    fn a_kind_repository_rejects_foreign_kinds() {
        let temporary = tempdir().unwrap();
        let store = LocalObjectStore::open(temporary.path()).unwrap();
        let engine = LibraryResourceEngine::new(store);
        // A tag pinned to the skills repository cannot publish under the
        // plugin repository id: the derivation check fails closed.
        let skills_repository = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::AgentSkill,
        );
        let error = engine
            .publish(PublishLibraryResourceRequest {
                repository_id: skills_repository.clone(),
                tag: tag(&plugin_repository()),
                observed: None,
                resources: set_of(&[plugin("review@review-market", "main")]),
                payloads: BTreeMap::new(),
                captured_with: CatalogCapturedWith {
                    app: "mallard".to_string(),
                    version: "test".to_string(),
                },
                published_by_origin: PortableCatalogOrigin {
                    origin_host_token: "a".repeat(64),
                    origin_replica_token: "b".repeat(64),
                },
                updated_at: 1,
            })
            .unwrap_err();
        assert!(
            error.contains("tag does not match its repository"),
            "{error}"
        );

        // A plugin entry cannot enter the skills repository even under the
        // skills repository's own valid tag: the manifest's homogeneity
        // validation refuses the set.
        let error = engine
            .publish(PublishLibraryResourceRequest {
                repository_id: skills_repository.clone(),
                tag: LibraryResourceTag {
                    schema_version: CATALOG_SCHEMA,
                    repository_id: skills_repository,
                    provider: ProviderKey::Codex,
                    resource_kind: CatalogResourceKindTag::AgentSkill,
                    created_at: 1,
                },
                observed: None,
                resources: set_of(&[plugin("review@review-market", "main")]),
                payloads: BTreeMap::new(),
                captured_with: CatalogCapturedWith {
                    app: "mallard".to_string(),
                    version: "test".to_string(),
                },
                published_by_origin: PortableCatalogOrigin {
                    origin_host_token: "a".repeat(64),
                    origin_replica_token: "b".repeat(64),
                },
                updated_at: 1,
            })
            .unwrap_err();
        assert!(
            error.contains("does not belong to this kind repository"),
            "{error}"
        );
    }
}
