//! Release proofs for the registry cutover (design §14.1 and §14.2): a build
//! with more registrations writes nothing at startup or on read-only use,
//! configuring a new provider touches only its own record, registering a new
//! portable coordinate writes nothing to cloud state, and publishing at that
//! coordinate leaves every existing object byte-identical while an older
//! registry parses it as inert.

use crate::project_sync::service::{
    config_view, create_provider_profile_with_repository, list_project_bindings_with_repository,
    recover_startup_state_with_repository,
};
use crate::sync_tests::thread_variants::harness::{
    codex_scope_key, harness_captured_with, publish_request, publish_with_head, ThreadSpec,
    TranscriptBuilder, VariantWorld,
};
use mallard_application::storage_setup::{
    commit_storage_setup_draft, create_storage_setup_draft, list_storage_setup_drafts,
    list_storages, storage_provider_descriptors, update_storage_setup_draft,
    UpdateStorageSetupDraftRequest,
};
use mallard_domain::{
    hex_lower, BoundedConfigObject, BoundedPortableObject, BundleRecipe, PortableContractResolver,
    RecipeEntry, ResourceDescriptor, ResourceId, SecretMap, StorageConfig, StorageId,
    StorageProviderConfig, StorageProviderKey, SyncScopeKey,
};
use mallard_infrastructure::bundle_engine::{self, BundleEngine, PublishExpectation};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::portable_resources::test_contracts::{
    imported_provenance, note_kind, review_twice_policy,
};
use mallard_infrastructure::portable_resources::PortableResourceRegistry;
use mallard_infrastructure::providers::codex::provider_key as codex_key;
use mallard_infrastructure::providers::AgentProviderRegistry;
use mallard_infrastructure::storage_core::{
    LocalObjectStore, ObjectPrefix, ObjectStore, REPOSITORY_PREFIX,
};
use mallard_infrastructure::storage_providers::local::local_storage_config;
use mallard_infrastructure::storage_providers::test_provider::{
    provider_key as test_storage_provider, TestStorageSettingsV1,
};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Every file below `root` with its content digest, keyed by relative path.
fn snapshot_files(root: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // The §1.2 exclusions: activity-log appends never hold provider state.
        if relative.starts_with("logs/") {
            continue;
        }
        let bytes = fs::read(entry.path()).unwrap();
        // Lock files are zero-byte synchronization artifacts, not records;
        // they are proven empty here so they cannot carry state.
        if relative.ends_with(".lock") {
            assert!(
                bytes.is_empty(),
                "{relative} is a lock file and must stay empty"
            );
            continue;
        }
        files.insert(relative, hex_lower(&Sha256::digest(&bytes)));
    }
    files
}

/// Every object under the repository prefix with its etag (the local store's
/// etag is the content digest, so equal etags mean equal bytes).
fn snapshot_store(store: &LocalObjectStore) -> BTreeMap<String, String> {
    let prefix = ObjectPrefix::parse(REPOSITORY_PREFIX).unwrap();
    let mut objects = BTreeMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = store.list(&prefix, cursor.as_deref(), 1_000).unwrap();
        for entry in page.entries {
            objects.insert(entry.key.as_str().to_string(), entry.etag);
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    objects
}

#[test]
fn a_larger_registry_writes_nothing_until_a_new_provider_is_configured() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("mallard");
    let store_root = temp.path().join("store");
    let codex_home = temp.path().join("codex-home");
    for path in [&home, &store_root, &codex_home] {
        fs::create_dir_all(path).unwrap();
    }
    let repository = LocalStateRepository::from_home_dir(&home).unwrap();

    // Schema-1 state written by the current registrations, including the
    // host identity a completed launch leaves behind.
    repository.load_or_create_host_identity().unwrap();
    let mut config = repository.load_config().unwrap();
    config.storages.push(local_storage_config(
        StorageId::parse("storage-local").unwrap(),
        "Local folder",
        &store_root,
    ));
    // A record from a build this one has never seen: it must load, list as
    // unavailable, and survive every edit untouched.
    config.storages.push(StorageConfig {
        id: StorageId::parse("storage-future").unwrap(),
        name: "Future provider".to_string(),
        provider: StorageProviderKey::parse("futurestore").unwrap(),
        provider_config: StorageProviderConfig::new(
            7,
            BoundedConfigObject::from_json(&serde_json::json!({"region": "moon-1"})).unwrap(),
            SecretMap::empty(),
        ),
    });
    repository.save_config(config).unwrap();
    create_provider_profile_with_repository(
        &repository,
        &codex_key(),
        "Codex",
        &codex_home.to_string_lossy(),
    )
    .unwrap();
    let before = snapshot_files(&home);
    assert!(
        before.len() >= 3,
        "fixture wrote config, identity and profiles"
    );

    // Start with an additional registration on every axis.
    let providers = AgentProviderRegistry::with_test_provider();
    let storage_registry = StorageProviderRegistry::with_test_provider();
    let contracts = PortableResourceRegistry::with_test_contracts();
    assert!(
        providers.descriptors().len() > AgentProviderRegistry::production().descriptors().len()
    );
    assert!(
        storage_registry.descriptors().len()
            > StorageProviderRegistry::production().descriptors().len()
    );
    assert!(
        contracts.resource_kind_descriptors().len()
            > PortableResourceRegistry::production()
                .resource_kind_descriptors()
                .len()
    );

    // Read-only listing, setup discovery, and startup recovery.
    let view = config_view(repository.load_config().unwrap()).unwrap();
    assert_eq!(view.storages.len(), 2);
    assert!(view.storages[0].available);
    assert!(
        !view.storages[1].available,
        "the unknown provider's record lists as unavailable"
    );
    assert_eq!(
        list_storages(&repository, &storage_registry).unwrap().len(),
        2
    );
    assert!(!storage_provider_descriptors(&storage_registry).is_empty());
    let (drafts, warnings) = list_storage_setup_drafts(&repository, &storage_registry).unwrap();
    assert!(drafts.is_empty() && warnings.is_empty());
    list_project_bindings_with_repository(&repository).unwrap();
    let setup_warnings = recover_startup_state_with_repository(&repository);
    assert!(setup_warnings.is_empty(), "{setup_warnings:?}");
    assert_eq!(
        snapshot_files(&home),
        before,
        "startup and read-only use with a larger registry performed no persistence write"
    );

    // Configure the new storage provider through the generic draft flow.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let draft =
        create_storage_setup_draft(&repository, &storage_registry, &test_storage_provider())
            .unwrap();
    let directory = temp.path().join("teststore-root");
    fs::create_dir_all(&directory).unwrap();
    let draft = update_storage_setup_draft(
        &repository,
        &storage_registry,
        UpdateStorageSetupDraftRequest {
            draft_id: draft.draft_id.clone(),
            revision: draft.revision,
            name: "Test store".to_string(),
            settings: BoundedConfigObject::encode(&TestStorageSettingsV1 {
                directory: directory.to_string_lossy().into_owned(),
                label: "proof".to_string(),
            })
            .unwrap(),
            secrets: BTreeMap::new(),
        },
    )
    .unwrap();
    let config_revision = repository.load_config().unwrap().revision;
    commit_storage_setup_draft(
        &repository,
        &storage_registry,
        runtime.handle(),
        &draft.draft_id,
        draft.revision,
        |_| Ok(()),
    )
    .unwrap();

    let after = snapshot_files(&home);
    let config_relative = repository
        .config_path()
        .strip_prefix(&home)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut changed = Vec::new();
    for (path, digest) in &before {
        if after.get(path) != Some(digest) {
            changed.push(path.clone());
        }
    }
    let mut added = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    added.sort();
    assert_eq!(
        changed,
        vec![config_relative.clone()],
        "only the owning document changed"
    );
    assert!(
        added.iter().all(|path| path.contains("setup")
            || path.contains("draft")
            || path.contains("journal")),
        "the only new files are the draft's own setup records: {added:?}"
    );

    let config = repository.load_config().unwrap();
    assert_eq!(config.revision, config_revision + 1);
    assert_eq!(config.storages.len(), 3);
    let (existing, new) = config.storages.split_at(2);
    let before_config = view.storages;
    assert_eq!(existing[0].id, before_config[0].storage_id);
    assert_eq!(existing[0].provider.as_str(), "local");
    assert_eq!(existing[1].id, before_config[1].storage_id);
    assert_eq!(existing[1].provider.as_str(), "futurestore");
    assert_eq!(existing[1].provider_config.version, 7);
    assert_eq!(
        existing[1].provider_config.settings.get_str("region"),
        Some("moon-1"),
        "the unknown record is intact"
    );
    assert_eq!(new[0].provider, test_storage_provider());
    assert_eq!(new[0].name, "Test store");
}

#[test]
fn registering_a_portable_coordinate_writes_nothing_and_publishing_at_it_preserves_old_objects() {
    let world = VariantWorld::new();
    let engine = world.engine();
    let origin = world.origin(&world.repo_a, "replica-a");
    let thread = ThreadSpec::new(
        "cloudthread-known",
        "provider-known",
        TranscriptBuilder::new("provider-known", "/tmp/project")
            .message("m-1", "user", "hello")
            .build(),
        origin.clone(),
        10,
    );
    let first = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let first_published = publish_with_head(&engine, first).unwrap();
    let before = snapshot_store(&world.store);
    assert!(
        before.len() >= 3,
        "revision 1 wrote head, manifest and payload objects"
    );

    // Registering the new coordinate is a compile-time fact: no write.
    let contracts = Arc::new(PortableResourceRegistry::with_test_contracts());
    assert_eq!(snapshot_store(&world.store), before);

    // Publish one resource at the new coordinate from the larger registry.
    let extended = BundleEngine::open_with_contracts(
        world.store.clone(),
        world.storage_id.clone(),
        contracts.clone(),
    )
    .unwrap();
    let note_id = ResourceId::parse("testprov:note:hello").unwrap();
    let mut recipe = BundleRecipe::default();
    recipe.entries.insert(
        note_id.clone(),
        RecipeEntry {
            resource_id: note_id.clone(),
            apply_policy: review_twice_policy(),
            required: false,
        },
    );
    let mut second = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin.clone(),
        11,
        Some(&first_published.snapshot.manifest),
        PublishExpectation::Match(first_published.head_token.clone()),
    );
    second.writable_scopes.clear();
    second.reviewed_scope_pins.clear();
    second.writable_scopes.insert(
        SyncScopeKey::Project,
        bundle_engine::ScopePublishInput {
            recipe,
            captured_with: harness_captured_with(),
            conversations: BTreeMap::new(),
            resources: BTreeMap::from([(
                note_id.clone(),
                ResourceDescriptor {
                    resource_id: note_id.clone(),
                    kind: note_kind(),
                    provider: None,
                    provider_resource_type: None,
                    display_name: "Proof note".to_string(),
                    provenance: imported_provenance("registry-proof"),
                    apply_policy: review_twice_policy(),
                    codec_version: 1,
                    attributes: BoundedPortableObject::from_json(&serde_json::json!({
                        "note": "hello"
                    }))
                    .unwrap(),
                },
            )]),
            retained_files: BTreeMap::new(),
            captured_files: BTreeMap::new(),
            directories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        },
    );
    second
        .reviewed_scope_pins
        .insert(SyncScopeKey::Project, bundle_engine::absent_scope_pin());
    let second_published = publish_with_head(&extended, second).unwrap();

    let after = snapshot_store(&world.store);
    let rewritten = before
        .iter()
        .filter(|(key, etag)| after.get(*key) != Some(*etag))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    assert!(
        rewritten
            .iter()
            .all(|key| key.contains("/_head.json") || key.contains("/_tags/")),
        "only the CAS head and the bundle tag record moved; every old object is byte-identical: {rewritten:?}"
    );
    assert!(
        after.len() > before.len(),
        "the new revision added objects instead of replacing any"
    );

    // The production registry parses the new coordinate as unavailable and
    // inert: its scope fails to resolve while the codex scope still does.
    let manifest = &second_published.snapshot.manifest;
    manifest.validate().unwrap();
    let production = PortableResourceRegistry::production();
    let resolved = manifest.resolve_scopes(production);
    assert!(resolved[&codex_scope_key()].is_ok());
    let inert = resolved[&SyncScopeKey::Project]
        .as_ref()
        .expect_err("the production registry cannot execute the test kind");
    assert!(inert.contains("testprov.note"), "{inert}");
    assert!(production.resource_kind_adapter(&note_kind()).is_none());
    // The larger registry activates it without any conversion.
    assert!(manifest
        .resolve_all(contracts.as_ref() as &dyn PortableContractResolver)
        .is_ok());

    // An older registry carries the inert scope through its own publish
    // byte-identically.
    let mut grown = thread;
    grown.transcript = TranscriptBuilder::new("provider-known", "/tmp/project")
        .message("m-1", "user", "hello")
        .message_with_parent("m-2", Some("m-1"), "assistant", "reply")
        .build();
    grown.updated_at = 12;
    let third = publish_request(
        world.bundle_id.clone(),
        &[grown],
        origin,
        12,
        Some(&second_published.snapshot.manifest),
        PublishExpectation::Match(second_published.head_token.clone()),
    );
    let third_published = publish_with_head(&engine, third).unwrap();
    assert_eq!(
        third_published
            .snapshot
            .manifest
            .scope(&SyncScopeKey::Project),
        second_published
            .snapshot
            .manifest
            .scope(&SyncScopeKey::Project)
    );
    assert!(third_published
        .snapshot
        .manifest
        .resolve_all(contracts.as_ref() as &dyn PortableContractResolver)
        .is_ok());
}
