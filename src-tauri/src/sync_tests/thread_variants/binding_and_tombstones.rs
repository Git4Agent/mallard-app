use super::harness::{analyzed_chain, chain, schema5_request, sha256, VariantSpec, VariantWorld};
use crate::project_sync_v3::bundle_engine::{PublishExpectation, PublishSchema5Request};
use crate::project_sync_v3::conversation_sync::{
    recover_unique_compatible_binding, review_conversation_tombstone, BindingRecoveryCandidate,
    BindingRecoveryResult, TombstoneDecision,
};
use crate::project_sync_v3::domain::{
    BundleRecipe, ConflictGroupId, Tombstone, TombstoneTarget, VariantId,
};
use std::collections::BTreeMap;

#[test]
fn missing_binding_recovers_only_one_unique_longest_compatible_ancestor() {
    let local = analyzed_chain(&chain(
        "local-thread",
        &[("m-a", "one"), ("m-b", "two"), ("m-c", "three")],
    ));
    let short = analyzed_chain(&chain("cloud-short", &[("m-a", "one")]));
    let longest = analyzed_chain(&chain("cloud-long", &[("m-a", "one"), ("m-b", "two")]));
    let unrelated = analyzed_chain(&chain("cloud-other", &[("m-x", "same title")]));
    let short_id = VariantId::parse("variant-short").unwrap();
    let longest_id = VariantId::parse("variant-long").unwrap();
    let unrelated_id = VariantId::parse("variant-unrelated").unwrap();

    assert_eq!(
        recover_unique_compatible_binding(
            Some(&local),
            &[
                BindingRecoveryCandidate {
                    variant_id: short_id.clone(),
                    chain: Some(&short),
                },
                BindingRecoveryCandidate {
                    variant_id: longest_id.clone(),
                    chain: Some(&longest),
                },
                BindingRecoveryCandidate {
                    variant_id: unrelated_id,
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
                    variant_id: longest_id,
                    chain: Some(&longest),
                },
                BindingRecoveryCandidate {
                    variant_id: VariantId::parse("variant-equal").unwrap(),
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
                variant_id: VariantId::parse("variant-title-only").unwrap(),
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
                variant_id: short_id,
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
fn tombstone_review_covers_unapproved_primary_final_stale_and_modified_cases() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "thread-v1",
        chain("thread-v1", &[("m-a", "base"), ("m-b1", "one")]),
        origin.clone(),
        10,
    );
    let v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "thread-v2",
        chain("thread-v2", &[("m-a", "base"), ("m-b2", "two")]),
        origin.clone(),
        10,
    );
    let published = world.publish(&[v1, v2], &v1_id, origin, 10);
    let group = &published.snapshot.manifest.conversation_groups
        [&ConflictGroupId::parse("group-main").unwrap()];

    assert_eq!(
        review_conversation_tombstone(group, &v2_id, "reviewed", "reviewed", false, None),
        TombstoneDecision::RejectUnapproved
    );
    assert_eq!(
        review_conversation_tombstone(group, &v1_id, "reviewed", "reviewed", true, None),
        TombstoneDecision::RejectPrimaryWithoutReplacement
    );
    assert_eq!(
        review_conversation_tombstone(group, &v1_id, "reviewed", "reviewed", true, Some(&v2_id),),
        TombstoneDecision::TombstoneVariant
    );
    assert_eq!(
        review_conversation_tombstone(group, &v2_id, "reviewed", "modified", true, None),
        TombstoneDecision::PreserveModifiedInSameGroup
    );
    assert_eq!(
        review_conversation_tombstone(
            group,
            &VariantId::parse("variant-missing").unwrap(),
            "reviewed",
            "reviewed",
            true,
            None,
        ),
        TombstoneDecision::RejectStaleDigest
    );

    let mut final_group = group.clone();
    final_group.variants.remove(&v2_id);
    assert_eq!(
        review_conversation_tombstone(&final_group, &v1_id, "reviewed", "reviewed", true, None,),
        TombstoneDecision::TombstoneGroup
    );
}

#[test]
fn reviewed_variant_and_group_tombstones_publish_once_and_retain_survivors() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let group_id = ConflictGroupId::parse("group-main").unwrap();
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        group_id.as_str(),
        v1_id.as_str(),
        "thread-v1",
        chain("thread-v1", &[("m-a", "base"), ("m-b1", "one")]),
        origin.clone(),
        10,
    );
    let v2 = VariantSpec::new(
        group_id.as_str(),
        v2_id.as_str(),
        "thread-v2",
        chain("thread-v2", &[("m-a", "base"), ("m-b2", "two")]),
        origin.clone(),
        10,
    );
    let initial = world.publish(&[v1.clone(), v2.clone()], &v1_id, origin.clone(), 10);
    let tombstone = Tombstone {
        target: TombstoneTarget::ConversationVariant {
            conflict_group_id: group_id.clone(),
            variant_id: v2_id.clone(),
        },
        last_sha256: Some(sha256(&v2.transcript)),
        deleted_at: 20,
    };
    let mut delete_variant = schema5_request(
        world.bundle_id.clone(),
        &[v1.clone()],
        &v1_id,
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    delete_variant
        .tombstones
        .insert(tombstone.canonical_key(), tombstone.clone());
    let deleted = world.engine().publish_schema5(delete_variant).unwrap();
    assert!(deleted.published);
    let group = &deleted.snapshot.manifest.conversation_groups[&group_id];
    assert_eq!(group.variants.keys().collect::<Vec<_>>(), vec![&v1_id]);
    assert_eq!(group.primary_variant_id, v1_id);

    let mut repeated = schema5_request(
        world.bundle_id.clone(),
        &[v1],
        &v1_id,
        origin.clone(),
        30,
        Some(&deleted.snapshot.manifest),
        PublishExpectation::Match(deleted.head_token),
    );
    repeated
        .tombstones
        .insert(tombstone.canonical_key(), tombstone);
    let repeated = world.engine().publish_schema5(repeated).unwrap();
    assert!(!repeated.published);
    assert_eq!(repeated.snapshot.head.generation, 2);

    let group_tombstone = Tombstone {
        target: TombstoneTarget::ConversationGroup {
            conflict_group_id: group_id.clone(),
        },
        last_sha256: Some("f".repeat(64)),
        deleted_at: 40,
    };
    let mut tombstones = repeated.snapshot.manifest.tombstones.clone();
    tombstones.remove(&format!("conversation-variant:{group_id}:{v2_id}"));
    tombstones.insert(group_tombstone.canonical_key(), group_tombstone);
    let delete_group = PublishSchema5Request {
        identity: repeated.snapshot.manifest.bundle.clone(),
        recipe: BundleRecipe::default(),
        captured_with: repeated.snapshot.manifest.captured_with.clone(),
        published_by_origin: origin,
        conversation_groups: BTreeMap::new(),
        resources: BTreeMap::new(),
        retained_files: BTreeMap::new(),
        captured_files: BTreeMap::new(),
        directories: BTreeMap::new(),
        tombstones,
        expected_head: PublishExpectation::Match(repeated.head_token),
        updated_at: 40,
    };
    let deleted_group = world.engine().publish_schema5(delete_group).unwrap();
    assert!(deleted_group.published);
    assert!(deleted_group
        .snapshot
        .manifest
        .conversation_groups
        .is_empty());
    assert!(deleted_group.snapshot.manifest.resources.is_empty());

    let mut modified_after_delete = VariantSpec::new(
        group_id.as_str(),
        v1_id.as_str(),
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
    let resurrect = schema5_request(
        world.bundle_id.clone(),
        &[modified_after_delete.clone()],
        &v1_id,
        world.origin(&world.repo_b, "replica-b"),
        50,
        Some(&deleted_group.snapshot.manifest),
        PublishExpectation::Match(deleted_group.head_token),
    );
    let resurrected = world.engine().publish_schema5(resurrect).unwrap();
    assert!(resurrected.published);
    assert_eq!(resurrected.snapshot.head.generation, 4);
    assert!(resurrected
        .snapshot
        .manifest
        .conversation_groups
        .contains_key(&group_id));
    assert!(!resurrected
        .snapshot
        .manifest
        .tombstones
        .values()
        .any(|tombstone| {
            matches!(
                &tombstone.target,
                TombstoneTarget::ConversationGroup { conflict_group_id }
                    if conflict_group_id == &group_id
            )
        }));
    let fixed_point = schema5_request(
        world.bundle_id.clone(),
        &[modified_after_delete],
        &v1_id,
        world.origin(&world.repo_b, "replica-b"),
        60,
        Some(&resurrected.snapshot.manifest),
        PublishExpectation::Match(resurrected.head_token),
    );
    assert!(
        !world
            .engine()
            .publish_schema5(fixed_point)
            .unwrap()
            .published
    );
}
