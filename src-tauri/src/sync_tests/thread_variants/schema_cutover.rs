use super::harness::{analyzed_chain, sha256, VariantWorld};
use crate::project_sync_v3::bundle_engine::{
    build_schema6_cutover_request, BundleObjectStore, CasExpectation, CasOutcome, ObjectKey,
    PublishBundleRequest, PublishExpectation,
};
use crate::project_sync_v3::conversation_sync::{
    recover_unique_compatible_binding, BindingRecoveryCandidate, BindingRecoveryResult,
};
use crate::project_sync_v3::domain::{
    deterministic_schema4_thread_id, BundleHead, BundleManifest, CapturedWith, Provider,
    ResourceId, BUNDLE_SCHEMA_V4, BUNDLE_SCHEMA_V6,
};
use crate::project_sync_v3::provider_capture::CapturedResources;

pub(super) const LEGACY_MANIFEST: &str =
    include_str!("../../../testdata/thread_variants/legacy/schema4-manifest.json");
pub(super) const LEGACY_TRANSCRIPT: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/valid-continuation.jsonl");

#[test]
fn committed_schema4_fixture_cutover_is_deterministic_readable_and_one_way() {
    let manifest: BundleManifest = serde_json::from_str(LEGACY_MANIFEST).unwrap();
    manifest.validate().unwrap();
    assert_eq!(manifest.schema_version, BUNDLE_SCHEMA_V4);
    let world = VariantWorld::new();
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let manifest_sha256 = sha256(&manifest_bytes);
    let manifest_key = format!(
        ".mallard/v1/repositories/{}/_manifests/7-commit-legacy-fixture.json",
        world.bundle_id
    );
    let transcript_key = format!(
        ".mallard/v1/repositories/{}/_uploads/upload-legacy/files/state/codex/sessions/legacy-thread.jsonl",
        world.bundle_id
    );
    world
        .store
        .put_immutable(&ObjectKey::parse(manifest_key).unwrap(), &manifest_bytes)
        .unwrap();
    world
        .store
        .put_immutable(
            &ObjectKey::parse(transcript_key).unwrap(),
            LEGACY_TRANSCRIPT,
        )
        .unwrap();
    let head = BundleHead {
        schema_version: BUNDLE_SCHEMA_V4,
        bundle_id: world.bundle_id.clone(),
        kind: manifest.bundle.kind,
        generation: manifest.generation,
        commit_id: manifest.commit_id.clone(),
        manifest_key: "_manifests/7-commit-legacy-fixture.json".to_string(),
        manifest_sha256,
        updated_at: manifest.updated_at,
    };
    head.validate().unwrap();
    let head_key = ObjectKey::parse(format!(
        ".mallard/v1/repositories/{}/_head.json",
        world.bundle_id
    ))
    .unwrap();
    assert!(matches!(
        world
            .store
            .compare_and_swap(
                &head_key,
                &CasExpectation::Absent,
                &serde_json::to_vec(&head).unwrap(),
            )
            .unwrap(),
        CasOutcome::Written { .. }
    ));

    let engine = world.engine();
    let legacy = engine.fetch(&world.bundle_id).unwrap();
    legacy.snapshot.validate().unwrap();
    assert_eq!(
        legacy.files.values().next().map(Vec::as_slice),
        Some(LEGACY_TRANSCRIPT)
    );
    let (_, token) = engine.read_head(&world.bundle_id).unwrap().unwrap();
    let publisher = world.origin(&world.repo_a, "replica-a");
    let request_a =
        build_schema6_cutover_request(&legacy, publisher.clone(), token.clone(), 1784678410)
            .unwrap();
    let request_b = build_schema6_cutover_request(&legacy, publisher, token, 1784678410).unwrap();
    assert_eq!(
        request_a.conversations.keys().collect::<Vec<_>>(),
        request_b.conversations.keys().collect::<Vec<_>>()
    );

    let legacy_resource = ResourceId::parse("codex:session:legacy-thread").unwrap();
    let expected_thread =
        deterministic_schema4_thread_id(&world.bundle_id, Provider::Codex, &legacy_resource)
            .unwrap();
    let migrated = &request_a.conversations[&expected_thread];
    assert!(migrated.created_by_origin.is_none());
    assert!(migrated.last_updated_by_origin.is_none());
    assert!(migrated.forked_from.is_none());
    assert_eq!(migrated.source_thread_id.as_deref(), Some("thread-fixture"));
    assert_eq!(
        request_a.captured_files.values().next().unwrap().bytes,
        LEGACY_TRANSCRIPT
    );
    let local_chain = analyzed_chain(LEGACY_TRANSCRIPT);
    assert_eq!(
        recover_unique_compatible_binding(
            Some(&local_chain),
            &[BindingRecoveryCandidate {
                thread_id: expected_thread.clone(),
                chain: Some(&local_chain),
            }],
        ),
        BindingRecoveryResult::Unique(expected_thread.clone())
    );

    let cutover = engine.publish_schema6(request_a).unwrap();
    assert!(cutover.published);
    assert_eq!(cutover.snapshot.head.schema_version, BUNDLE_SCHEMA_V6);
    assert_eq!(cutover.snapshot.manifest.schema_version, BUNDLE_SCHEMA_V6);
    assert!(cutover
        .snapshot
        .manifest
        .resources
        .get(&legacy_resource)
        .is_none());
    assert!(cutover
        .snapshot
        .manifest
        .conversations
        .contains_key(&expected_thread));
    let fetched = engine.fetch(&world.bundle_id).unwrap();
    fetched.snapshot.validate().unwrap();
    assert!(fetched
        .files
        .values()
        .any(|bytes| bytes == LEGACY_TRANSCRIPT));

    let old_writer = engine.publish(PublishBundleRequest {
        identity: cutover.snapshot.manifest.bundle.clone(),
        recipe: cutover.snapshot.manifest.recipe.clone(),
        captured_with: CapturedWith {
            app_version: "schema-4-writer".to_string(),
            codex_version: None,
            claude_version: None,
            codec_versions: Default::default(),
        },
        captured: CapturedResources::default(),
        expected_head: PublishExpectation::Match(cutover.head_token),
        updated_at: 1784678420,
    });
    assert!(old_writer
        .unwrap_err()
        .contains("conversation-thread publication path"));
}
