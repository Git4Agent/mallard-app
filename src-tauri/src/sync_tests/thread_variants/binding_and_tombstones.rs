use super::harness::{analyzed_chain, chain, publish_request, sha256, ThreadSpec, VariantWorld};
use crate::project_sync_v3::bundle_engine::{PublishBundleRequest, PublishContent};
use crate::project_sync_v3::conversation_sync::{
    recover_unique_compatible_binding, review_conversation_tombstone, BindingRecoveryCandidate,
    BindingRecoveryResult, TombstoneDecision,
};
use crate::project_sync_v3::domain::{BundleRecipe, ThreadId, Tombstone, TombstoneTarget};

#[test]
fn missing_binding_recovers_only_one_unique_longest_compatible_ancestor() {
    let local = analyzed_chain(&chain(
        "local-thread",
        &[("m-a", "one"), ("m-b", "two"), ("m-c", "three")],
    ));
    let short = analyzed_chain(&chain("cloud-short", &[("m-a", "one")]));
    let longest = analyzed_chain(&chain("cloud-long", &[("m-a", "one"), ("m-b", "two")]));
    let unrelated = analyzed_chain(&chain("cloud-other", &[("m-x", "same title")]));
    let short_id = ThreadId::parse("thread-short").unwrap();
    let longest_id = ThreadId::parse("thread-long").unwrap();
    let unrelated_id = ThreadId::parse("thread-unrelated").unwrap();

    assert_eq!(
        recover_unique_compatible_binding(
            Some(&local),
            &[
                BindingRecoveryCandidate {
                    thread_id: short_id.clone(),
                    chain: Some(&short),
                },
                BindingRecoveryCandidate {
                    thread_id: longest_id.clone(),
                    chain: Some(&longest),
                },
                BindingRecoveryCandidate {
                    thread_id: unrelated_id,
                    chain: Some(&unrelated),
                },
            ],
        ),
        BindingRecoveryResult::Unique(longest_id.clone())
    );

    assert_eq!(
        recover_unique_compatible_binding(
            Some(&local),
            &[
                BindingRecoveryCandidate {
                    thread_id: longest_id,
                    chain: Some(&longest),
                },
                BindingRecoveryCandidate {
                    thread_id: ThreadId::parse("thread-equal").unwrap(),
                    chain: Some(&longest),
                },
            ],
        ),
        BindingRecoveryResult::Ambiguous
    );
    assert_eq!(
        recover_unique_compatible_binding(
            Some(&local),
            &[BindingRecoveryCandidate {
                thread_id: ThreadId::parse("thread-title-only").unwrap(),
                chain: Some(&unrelated),
            }],
        ),
        BindingRecoveryResult::Missing,
        "display titles and provider IDs are intentionally absent from recovery evidence",
    );
    assert_eq!(
        recover_unique_compatible_binding(
            Some(&local),
            &[BindingRecoveryCandidate {
                thread_id: short_id,
                chain: None,
            }],
        ),
        BindingRecoveryResult::Unknown
    );
    assert_eq!(
        recover_unique_compatible_binding(None, &[]),
        BindingRecoveryResult::Unknown
    );
}

#[test]
fn tombstone_review_covers_unapproved_stale_and_modified_cases() {
    assert_eq!(
        review_conversation_tombstone(true, "reviewed", "reviewed", false),
        TombstoneDecision::RejectUnapproved
    );
    assert_eq!(
        review_conversation_tombstone(true, "reviewed", "modified", true),
        TombstoneDecision::PreserveModified,
        "a modified copy always survives a tombstone race",
    );
    assert_eq!(
        review_conversation_tombstone(false, "reviewed", "reviewed", true),
        TombstoneDecision::RejectStaleDigest
    );
    assert_eq!(
        review_conversation_tombstone(true, "reviewed", "reviewed", true),
        TombstoneDecision::TombstoneThread
    );
}

#[test]
fn reviewed_thread_tombstones_publish_once_and_retain_survivors() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let t1_id = ThreadId::parse("thread-t1").unwrap();
    let t2_id = ThreadId::parse("thread-t2").unwrap();
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
        target: TombstoneTarget::ConversationThread {
            thread_id: t2_id.clone(),
        },
        last_sha256: Some(sha256(&t2.transcript)),
        deleted_at: 20,
    };
    let mut delete_thread = publish_request(
        world.bundle_id.clone(),
        &[t1.clone()],
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        Some((initial.snapshot.head.clone(), initial.head_token)),
    );
    delete_thread
        .content
        .tombstones
        .insert(tombstone.canonical_key(), tombstone.clone());
    let deleted = world.engine().publish(delete_thread).unwrap();
    assert!(deleted.published);
    assert_eq!(
        deleted
            .snapshot
            .manifest
            .conversations
            .keys()
            .collect::<Vec<_>>(),
        vec![&t1_id]
    );

    let mut repeated = publish_request(
        world.bundle_id.clone(),
        &[t1.clone()],
        origin.clone(),
        30,
        Some(&deleted.snapshot.manifest),
        Some((deleted.snapshot.head.clone(), deleted.head_token)),
    );
    repeated
        .content
        .tombstones
        .insert(tombstone.canonical_key(), tombstone.clone());
    let repeated = world.engine().publish(repeated).unwrap();
    assert!(!repeated.published);
    assert_eq!(repeated.snapshot.head.latest.generation, 2);

    let final_tombstone = Tombstone {
        target: TombstoneTarget::ConversationThread {
            thread_id: t1_id.clone(),
        },
        last_sha256: Some(sha256(&t1.transcript)),
        deleted_at: 40,
    };
    let mut tombstones = repeated.snapshot.manifest.tombstones.clone();
    tombstones.insert(final_tombstone.canonical_key(), final_tombstone);
    let delete_all = PublishBundleRequest {
        identity: repeated.snapshot.manifest.bundle.clone(),
        recipe: BundleRecipe::default(),
        captured_with: repeated.snapshot.manifest.captured_with.clone(),
        published_by_origin: origin,
        content: PublishContent {
            tombstones,
            ..PublishContent::default()
        },
        observed_head: Some((repeated.snapshot.head.clone(), repeated.head_token)),
        updated_at: 40,
    };
    let deleted_all = world.engine().publish(delete_all).unwrap();
    assert!(deleted_all.published);
    assert!(deleted_all.snapshot.manifest.conversations.is_empty());
    assert!(deleted_all.snapshot.manifest.resources.is_empty());

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
        Some((deleted_all.snapshot.head.clone(), deleted_all.head_token)),
    );
    let resurrected = world.engine().publish(resurrect).unwrap();
    assert!(resurrected.published);
    assert_eq!(resurrected.snapshot.head.latest.generation, 4);
    assert!(resurrected
        .snapshot
        .manifest
        .conversations
        .contains_key(&t1_id));
    assert!(!resurrected
        .snapshot
        .manifest
        .tombstones
        .values()
        .any(|tombstone| {
            matches!(
                &tombstone.target,
                TombstoneTarget::ConversationThread { thread_id }
                    if thread_id == &t1_id
            )
        }));
    let fixed_point = publish_request(
        world.bundle_id.clone(),
        &[modified_after_delete],
        world.origin(&world.repo_b, "replica-b"),
        60,
        Some(&resurrected.snapshot.manifest),
        Some((resurrected.snapshot.head.clone(), resurrected.head_token)),
    );
    assert!(!world.engine().publish(fixed_point).unwrap().published);
}
