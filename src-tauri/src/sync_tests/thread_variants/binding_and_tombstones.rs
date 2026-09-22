use super::harness::{
    chain, codex_scope, codex_scope_key, harness_captured_with, publish_request, publish_with_head,
    scope_pin, sha256, ThreadSpec, VariantWorld,
};
use mallard_domain::portable::contracts;
use mallard_domain::{BundleRecipe, CloudThreadId, Tombstone};
use mallard_infrastructure::bundle_engine::{
    PublishBundleRequest, PublishExpectation, ScopePublishInput,
};
use std::collections::BTreeMap;

#[test]
fn reviewed_thread_tombstones_publish_once_and_retain_survivors() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let t1_id = CloudThreadId::parse("cloudthread-t1").unwrap();
    let t2_id = CloudThreadId::parse("cloudthread-t2").unwrap();
    let t1 = ThreadSpec::new(
        t1_id.as_str(),
        "thread-v1",
        chain("thread-v1", &[("m-a", "base"), ("m-b1", "one")]),
        origin.clone(),
        10,
    );
    let t2 = ThreadSpec::new(
        t2_id.as_str(),
        "thread-v2",
        chain("thread-v2", &[("m-a", "base"), ("m-b2", "two")]),
        origin.clone(),
        10,
    );
    let initial = world.publish(&[t1.clone(), t2.clone()], origin.clone(), 10);
    let tombstone = Tombstone {
        target: contracts::tombstone_conversation_thread(&t2_id).unwrap(),
        last_sha256: Some(sha256(&t2.transcript)),
        deleted_at: 20,
    };
    let mut delete_thread = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&t1),
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(Box::new(initial.snapshot.clone()), initial.head_token),
    );
    delete_thread
        .writable_scopes
        .get_mut(&codex_scope_key())
        .unwrap()
        .tombstones
        .insert(tombstone.structural_key(), tombstone.clone());
    let engine = world.engine();
    let deleted = publish_with_head(&engine, delete_thread).unwrap();
    assert!(deleted.published);
    assert_eq!(
        codex_scope(&deleted.snapshot.manifest)
            .conversations
            .keys()
            .collect::<Vec<_>>(),
        vec![&t1_id]
    );

    let mut repeated = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&t1),
        origin.clone(),
        30,
        Some(&deleted.snapshot.manifest),
        PublishExpectation::Match(Box::new(deleted.snapshot.clone()), deleted.head_token),
    );
    repeated
        .writable_scopes
        .get_mut(&codex_scope_key())
        .unwrap()
        .tombstones
        .insert(tombstone.structural_key(), tombstone.clone());
    let repeated = publish_with_head(&engine, repeated).unwrap();
    assert!(!repeated.published);
    assert_eq!(repeated.snapshot.head.revision, 2);

    let final_tombstone = Tombstone {
        target: contracts::tombstone_conversation_thread(&t1_id).unwrap(),
        last_sha256: Some(sha256(&t1.transcript)),
        deleted_at: 40,
    };
    let mut tombstones = codex_scope(&repeated.snapshot.manifest).tombstones.clone();
    tombstones.insert(final_tombstone.structural_key(), final_tombstone);
    let delete_all = PublishBundleRequest {
        identity: repeated.snapshot.manifest.bundle.clone(),
        published_by_origin: origin,
        reviewed_scope_pins: BTreeMap::from([(
            codex_scope_key(),
            scope_pin(Some(&repeated.snapshot.manifest), &codex_scope_key()),
        )]),
        writable_scopes: BTreeMap::from([(
            codex_scope_key(),
            ScopePublishInput {
                recipe: BundleRecipe::default(),
                captured_with: harness_captured_with(),
                tombstones,
                ..ScopePublishInput::default()
            },
        )]),
        expected_head: PublishExpectation::Match(
            Box::new(repeated.snapshot.clone()),
            repeated.head_token,
        ),
        updated_at: 40,
    };
    let deleted_all = publish_with_head(&engine, delete_all).unwrap();
    assert!(deleted_all.published);
    assert_eq!(deleted_all.snapshot.manifest.all_conversations().count(), 0);
    assert_eq!(deleted_all.snapshot.manifest.all_resources().count(), 0);

    let mut modified_after_delete = ThreadSpec::new(
        t1_id.as_str(),
        "thread-v1",
        chain(
            "thread-v1",
            &[
                ("m-a", "base"),
                ("m-b1", "one"),
                ("m-c1", "modified locally"),
            ],
        ),
        world.origin(&world.repo_b, "replica-b"),
        50,
    );
    modified_after_delete.created_at = 1;
    let resurrect = publish_request(
        world.bundle_id.clone(),
        &[modified_after_delete.clone()],
        world.origin(&world.repo_b, "replica-b"),
        50,
        Some(&deleted_all.snapshot.manifest),
        PublishExpectation::Match(
            Box::new(deleted_all.snapshot.clone()),
            deleted_all.head_token,
        ),
    );
    let resurrected = publish_with_head(&engine, resurrect).unwrap();
    assert!(resurrected.published);
    assert_eq!(resurrected.snapshot.head.revision, 4);
    assert!(codex_scope(&resurrected.snapshot.manifest)
        .conversations
        .contains_key(&t1_id));
    assert!(!resurrected
        .snapshot
        .manifest
        .all_tombstones()
        .any(|(_, _, tombstone)| {
            tombstone.target.kind.as_str() == contracts::CONVERSATION_THREAD
                && tombstone.target.identity.get_str("thread_id") == Some(t1_id.as_str())
        }));
    let fixed_point = publish_request(
        world.bundle_id.clone(),
        &[modified_after_delete],
        world.origin(&world.repo_b, "replica-b"),
        60,
        Some(&resurrected.snapshot.manifest),
        PublishExpectation::Match(
            Box::new(resurrected.snapshot.clone()),
            resurrected.head_token,
        ),
    );
    assert!(!engine.publish(fixed_point).unwrap().stored());
}
