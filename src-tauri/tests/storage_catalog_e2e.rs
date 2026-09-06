//! Deterministic end-to-end coverage for the storage-scoped Library.
//!
//! Unlike the opt-in Codex conversation test, this test never starts a model
//! or provider process. It exercises the production Library service against
//! two isolated Mallard/Codex homes and one real local-folder object store.

mod support;

use mallard_domain::StorageId;
use mallard_infrastructure::storage_providers::local::local_storage_config;
use support::catalog_flow::{run_catalog_flow_with_storage_reset, CatalogFlowInputs};

#[test]
fn skills_and_codex_pets_round_trip_update_and_preserve_divergence() {
    let temporary = tempfile::tempdir().expect("create Library integration root");
    let root = temporary.path();
    let machine_home_a = root.join("machine-a");
    let machine_home_b = root.join("machine-b");
    let user_home_a = root.join("user-a");
    let user_home_b = root.join("user-b");
    let codex_home_a = root.join("myconf1");
    let codex_home_b = root.join("myconf2");
    let storage_root = root.join("mystorage1");
    for directory in [
        &machine_home_a,
        &machine_home_b,
        &user_home_a,
        &user_home_b,
        &codex_home_a,
        &codex_home_b,
        &storage_root,
    ] {
        std::fs::create_dir_all(directory).expect("create isolated Library directory");
    }
    let auth_a = b"{\"test_token\":\"catalog-e2e-auth-a-must-not-travel\"}\n";
    let auth_b = b"{\"test_token\":\"catalog-e2e-auth-b-must-not-travel\"}\n";
    std::fs::write(codex_home_a.join("auth.json"), auth_a).expect("write fake myconf1 auth");
    std::fs::write(codex_home_b.join("auth.json"), auth_b).expect("write fake myconf2 auth");

    let storage = local_storage_config(
        StorageId::parse("catalog-e2e-local").expect("valid test storage id"),
        "Catalog E2E local storage",
        &storage_root,
    );
    let privacy_needles = [
        machine_home_a.to_string_lossy().as_bytes().to_vec(),
        machine_home_b.to_string_lossy().as_bytes().to_vec(),
        user_home_a.to_string_lossy().as_bytes().to_vec(),
        user_home_b.to_string_lossy().as_bytes().to_vec(),
        codex_home_a.to_string_lossy().as_bytes().to_vec(),
        codex_home_b.to_string_lossy().as_bytes().to_vec(),
        storage_root.to_string_lossy().as_bytes().to_vec(),
        b"catalog-e2e-auth-a-must-not-travel".to_vec(),
        b"catalog-e2e-auth-b-must-not-travel".to_vec(),
    ];

    let report = run_catalog_flow_with_storage_reset(CatalogFlowInputs {
        machine_home_a: &machine_home_a,
        machine_home_b: &machine_home_b,
        user_home_a: &user_home_a,
        user_home_b: &user_home_b,
        codex_home_a: &codex_home_a,
        codex_home_b: &codex_home_b,
        storage: &storage,
        privacy_needles: &privacy_needles,
    });

    assert_eq!(report.repository_count, 3);
    assert_eq!(report.kind_repository_revisions, vec![5, 5, 7]);
    assert_eq!(report.native_provider_process_calls, 0);
    assert!(report.injected_inventory_calls > 0);
    assert_eq!(
        std::fs::read(codex_home_a.join("auth.json")).expect("read fake myconf1 auth"),
        auth_a
    );
    assert_eq!(
        std::fs::read(codex_home_b.join("auth.json")).expect("read fake myconf2 auth"),
        auth_b
    );
}
