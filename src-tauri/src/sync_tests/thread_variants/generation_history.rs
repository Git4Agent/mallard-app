//! Generation history over portable thread metadata: whole-snapshot
//! equality is answered from verified refs alone, titles come from the
//! manifest, and a republished older snapshot is recognized as logically
//! identical while remaining a distinct immutable manifest.

use super::harness::{chain, ThreadSpec, VariantWorld};
use crate::project_sync_v3::bundle_engine::BundleResourceChangeState;
use crate::project_sync_v3::domain::ThreadId;

#[test]
fn republished_snapshot_compares_equal_without_manifest_reads() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let original = ThreadSpec::new(
        "thread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "base")]),
        origin.clone(),
        10,
    );
    let mut extended = original.clone();
    extended.transcript = chain("source-thread", &[("m-a", "base"), ("m-b", "more")]);
    extended.updated_at = 20;

    // G1 -> G2 changes the thread; G3 intentionally republishes G1's exact
    // logical content after the intervening change.
    let first = world.publish(&[original.clone()], origin.clone(), 10);
    let second = world.publish(&[extended], origin.clone(), 20);
    let third = world.publish(&[original], origin.clone(), 30);
    assert_eq!(third.snapshot.head.latest.generation, 3);

    let engine = world.engine();
    let comparison = engine
        .compare_generations(&world.bundle_id, 1, 3, true)
        .unwrap();
    assert!(comparison.same_snapshot);
    assert!(comparison.resource_changes.is_empty());
    assert_eq!(
        comparison.left.version.snapshot_content_sha256,
        comparison.right.version.snapshot_content_sha256
    );
    // Same snapshot, distinct immutable manifests: logical equality is not
    // manifest identity.
    assert_ne!(
        comparison.left.version.manifest_sha256,
        comparison.right.version.manifest_sha256
    );

    let detailed = engine
        .compare_generations(&world.bundle_id, 1, 2, true)
        .unwrap();
    assert!(!detailed.same_snapshot);
    assert_eq!(detailed.changed, 1);
    // A changed conversation is reported as changed — digest inequality
    // never claims prefix or divergence classification on its own.
    assert_eq!(
        detailed.resource_changes[0].state,
        BundleResourceChangeState::Changed
    );

    let _ = (first, second);
}

#[test]
fn title_only_change_is_a_resource_change_with_both_titles() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let mut titled = ThreadSpec::new(
        "thread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "base")]),
        origin.clone(),
        10,
    );
    titled.title = "Before rename".to_string();
    world.publish(&[titled.clone()], origin.clone(), 10);
    titled.title = "After rename".to_string();
    world.publish(&[titled], origin.clone(), 20);

    let comparison = world
        .engine()
        .compare_generations(&world.bundle_id, 1, 2, true)
        .unwrap();
    assert!(!comparison.same_snapshot);
    assert_eq!(comparison.changed, 1);
    let row = &comparison.resource_changes[0];
    assert_eq!(row.state, BundleResourceChangeState::Changed);
    assert_eq!(row.left_thread_title.as_deref(), Some("Before rename"));
    assert_eq!(row.right_thread_title.as_deref(), Some("After rename"));
}

#[test]
fn generation_listing_carries_titles_free_delta_counts() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let t1 = ThreadSpec::new(
        "thread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "base")]),
        origin.clone(),
        10,
    );
    let t2 = ThreadSpec::new(
        "thread-v2",
        "second-source",
        chain("second-source", &[("m-a", "other")]),
        origin.clone(),
        10,
    );
    world.publish(&[t1.clone()], origin.clone(), 10);
    world.publish(&[t1, t2], origin.clone(), 20);

    let page = world
        .engine()
        .list_reachable_generations(&world.bundle_id, None, 10)
        .unwrap();
    assert_eq!(page.generations.len(), 2);
    let latest = &page.generations[0];
    assert_eq!(latest.record.delta.added_resources, 1);
    assert_eq!(latest.record.delta.changed_resources, 0);
    // The delta is counts only; stored titles live in the manifest's
    // conversation metadata, never in history records.
    let serialized = serde_json::to_string(&latest.record).unwrap();
    assert!(!serialized.contains("Thread test conversation"));
    assert!(!serialized.contains(ThreadId::parse("thread-v2").unwrap().as_str()));
}
