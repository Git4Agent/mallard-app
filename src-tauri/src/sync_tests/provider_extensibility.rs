//! Registration-driven extensibility proofs (plan §16.6): the test-only
//! third provider enters common discovery, capture, and codec analysis
//! through `ProviderRegistry::with_test_provider()` alone, and a structurally
//! valid scope of a provider this binary has never registered survives a
//! common publish byte-identical.

use crate::project_sync::bundle_engine::{self, CapturedBundleFile, PublishExpectation};
use crate::project_sync::domain::{provider_state_path_prefix, ApplyPolicy, BundleRecipe};
use crate::project_sync::provider_capture::{self, CaptureRequest, CaptureResourceKind};
use crate::project_sync::providers::{test_provider, ProviderRegistry};
use crate::sync_tests::thread_variants::harness::{
    codex_scope_key, harness_captured_with, publish_request, publish_with_head, ThreadSpec,
    TranscriptBuilder, VariantWorld,
};
use mallard_domain::{
    LogicalPath, Provenance, ProviderKey, RecipeEntry, ResourceDescriptor, ResourceId,
    ResourceKind, ResourceTypeKey, SyncScopeKey,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[test]
fn third_provider_registers_through_descriptors_alone() {
    let production = ProviderRegistry::production()
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.key.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(production, vec!["claude".to_string(), "codex".to_string()]);

    let with_test = ProviderRegistry::with_test_provider()
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.key.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        with_test,
        vec![
            "claude".to_string(),
            "codex".to_string(),
            "testprov".to_string(),
        ],
        "registering the third provider is one registry entry, no common-flow change"
    );
}

#[test]
fn third_provider_sessions_flow_through_common_discovery_and_capture() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("project");
    let provider_home = temp.path().join("testprov-home");
    let sessions = provider_home.join("sessions");
    fs::create_dir_all(&project_root).unwrap();
    fs::create_dir_all(&sessions).unwrap();
    let canonical_project = fs::canonicalize(&project_root).unwrap();

    let session_id = "3f1c2b4a-0000-4000-8000-00000000aa01";
    let cwd = canonical_project.to_string_lossy().into_owned();
    let transcript = [
        serde_json::json!({
            "type": "user",
            "uuid": "u-1",
            "parentUuid": null,
            "sessionId": session_id,
            "cwd": cwd,
            "timestamp": "2026-08-07T00:00:00.000Z",
            "version": "2.0.0",
            "message": {"role": "user", "content": [{"type": "text", "text": "hello"}]}
        }),
        serde_json::json!({
            "type": "assistant",
            "uuid": "a-1",
            "parentUuid": "u-1",
            "sessionId": session_id,
            "cwd": cwd,
            "timestamp": "2026-08-07T00:00:01.000Z",
            "version": "2.0.0",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "hi"}]}
        }),
    ]
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(sessions.join(format!("{session_id}.jsonl")), &transcript).unwrap();

    let registry = ProviderRegistry::with_test_provider();
    let request = CaptureRequest {
        project_root: project_root.clone(),
        provider_homes: BTreeMap::from([(test_provider::provider_key(), provider_home)]),
        excluded_project_roots: Vec::new(),
        include_project_content: false,
        excluded_content_roots: Vec::new(),
    };
    let discovery = provider_capture::discover_full(&registry, &request).unwrap();
    let inventory = discovery.project_inventory();
    let candidates = inventory
        .resources
        .iter()
        .filter(|candidate| candidate.kind == CaptureResourceKind::Conversation)
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), 1, "warnings: {:?}", inventory.warnings);
    let candidate = candidates[0];
    assert_eq!(
        candidate.provider.as_ref(),
        Some(&test_provider::provider_key()),
        "common discovery attributes the session to the registered adapter"
    );

    let selected = BTreeSet::from([candidate.resource_id.clone()]);
    let captured =
        provider_capture::capture_from_discovery(&registry, &request, &discovery, &selected)
            .unwrap();
    assert_eq!(captured.conversations.len(), 1);
    let conversation = captured.conversations.values().next().unwrap();
    assert_eq!(conversation.provider, test_provider::provider_key());
    assert_eq!(
        conversation.lineage_codec,
        test_provider::TEST_PROVIDER_CODEC_KEY,
        "the adapter's own codec key travels with the capture"
    );
    assert_eq!(conversation.source_thread_id.as_deref(), Some(session_id));
    assert_eq!(conversation.record_count, 2);
    assert!(
        conversation.lineage.chain().is_some(),
        "the shared codec proves the third provider's lineage"
    );
}

#[test]
fn unregistered_provider_scope_is_carried_unchanged_through_a_publish() {
    let world = VariantWorld::new();
    let engine = world.engine();
    let origin = world.origin(&world.repo_a, "replica-a");

    // Revision 1: one codex conversation plus a structurally valid scope of
    // a provider no adapter in this binary recognizes.
    let future = ProviderKey::parse("futureprov").unwrap();
    let future_scope_key = SyncScopeKey::Provider(future.clone());
    let future_resource = ResourceId::parse("futureprov:state:notes").unwrap();
    let future_path =
        LogicalPath::parse(format!("{}notes.md", provider_state_path_prefix(&future))).unwrap();
    let thread = ThreadSpec::new(
        "thread-known",
        "provider-known",
        TranscriptBuilder::new("provider-known", "/tmp/project")
            .message("m-1", "user", "hello")
            .build(),
        origin.clone(),
        10,
    );
    let mut first = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let mut future_recipe = BundleRecipe::default();
    future_recipe.entries.insert(
        future_resource.clone(),
        RecipeEntry {
            resource_id: future_resource.clone(),
            apply_policy: ApplyPolicy::SafeFile,
            required: false,
        },
    );
    first.writable_scopes.insert(
        future_scope_key.clone(),
        bundle_engine::ScopePublishInput {
            recipe: future_recipe,
            captured_with: harness_captured_with(),
            conversations: BTreeMap::new(),
            resources: BTreeMap::from([(
                future_resource.clone(),
                ResourceDescriptor {
                    resource_id: future_resource.clone(),
                    kind: ResourceKind::ProviderState,
                    provider: Some(future.clone()),
                    provider_resource_type: Some(ResourceTypeKey::parse("state").unwrap()),
                    display_name: "Future provider notes".to_string(),
                    provenance: Provenance::Unknown,
                    apply_policy: ApplyPolicy::SafeFile,
                    relative_cwd: None,
                    codec_version: 1,
                    metadata: BTreeMap::new(),
                },
            )]),
            retained_files: BTreeMap::new(),
            captured_files: BTreeMap::from([(
                future_path.clone(),
                CapturedBundleFile {
                    resource_id: future_resource.clone(),
                    bytes: b"future provider state\n".to_vec(),
                    source_mtime: 10,
                    mode: Some(0o600),
                },
            )]),
            directories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        },
    );
    first
        .reviewed_scope_pins
        .insert(future_scope_key.clone(), bundle_engine::absent_scope_pin());
    let first_published = publish_with_head(&engine, first).unwrap();
    let first_future = first_published
        .snapshot
        .manifest
        .scope(&future_scope_key)
        .expect("first publish records the unknown provider scope")
        .clone();

    // Revision 2: a codex-only publish carries the unknown scope forward
    // byte-identically — same revision, digest, recipe, and file entry.
    let mut grown = thread;
    grown.transcript = TranscriptBuilder::new("provider-known", "/tmp/project")
        .message("m-1", "user", "hello")
        .message_with_parent("m-2", Some("m-1"), "assistant", "reply")
        .build();
    grown.updated_at = 11;
    let second = publish_request(
        world.bundle_id.clone(),
        &[grown],
        origin,
        11,
        Some(&first_published.snapshot.manifest),
        PublishExpectation::Match(first_published.head_token.clone()),
    );
    assert!(
        !second.writable_scopes.contains_key(&future_scope_key),
        "the codex publish must not name the unknown scope writable"
    );
    let second_published = publish_with_head(&engine, second).unwrap();
    let carried = second_published
        .snapshot
        .manifest
        .scope(&future_scope_key)
        .expect("the unknown provider scope survives an unrelated publish");
    assert_eq!(carried, &first_future);
    assert!(
        second_published
            .snapshot
            .manifest
            .scope(&codex_scope_key())
            .is_some_and(|scope| scope.revision
                > first_published
                    .snapshot
                    .manifest
                    .scope(&codex_scope_key())
                    .unwrap()
                    .revision),
        "the codex scope advanced while the unknown scope stood still"
    );
}
