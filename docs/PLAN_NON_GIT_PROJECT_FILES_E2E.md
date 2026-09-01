# Rust E2E Test: Non-Git Project Files

- **Status:** Proposed
- **Scope:** Current `ProjectSync` path, schema 1
- **Test binary:** `src-tauri/tests/project_files_e2e.rs`
- **Test cases:** Complex round trip and divergent Pull recovery

## 1. Objective

Add one deterministic Rust integration-test binary with two tests that prove
reviewed project-file sync from one non-Git project folder to another:

```text
Machine A project
    -> explicit Scan
    -> reviewed Push
    -> real local-folder storage
    -> reviewed Pull plan
    -> approved Apply
    -> Machine B project
```

The test must exercise production setup, scanning, publication, manifest
inspection, restore planning, approval, filesystem mutation, and convergence.
It must not use a real user home, a real provider home, a live cloud service,
or a provider process.

This is not a browser test. Existing frontend integration tests continue to
verify the Project files review controls. This test owns the filesystem,
storage, and application-service boundary.

## 2. Product properties proved

The test proves these properties in one end-to-end flow:

1. Opening Push does not discover or publish ordinary project files.
2. Project-file discovery occurs only after explicit Scan.
3. Scan is read-only for remote state and saved project-file preferences.
4. Push publishes only reviewed project resources and the exact reviewed
   bytes for a multi-level tree.
5. Portable state does not contain Machine A absolute paths or excluded bytes.
6. Pull creates typed project-file actions that require explicit approval.
7. Apply creates required directories and writes the files under Machine B's
   different absolute project root.
8. Independent empty directories survive the round trip.
9. An explicitly unselected local subtree does not travel.
10. `.mallardignore` rules keep an ignored subtree and matching scratch file
    out of discovery and storage.
11. Text, JSON, TypeScript, Markdown, and binary payloads retain exact bytes.
12. A second Pull has no outstanding or divergent project-file row.
13. A scan on Machine B reports the restored entries as synced with the same
    stable resource IDs.
14. Inventory distinguishes `storage_ahead`, `local_ahead`, `synced`, and
    `diverged` files against the reviewed base.
15. Pull review selects only `storage_ahead` and `diverged` files.
16. A storage-ahead file fast-forwards without a preservation copy.
17. A diverged file keeps the local bytes in a visible sibling before storage
    bytes replace the original path.
18. A local-ahead file and a synced file remain untouched by the Pull.
19. A local edit after planning blocks the stale action without overwriting or
    copying unreviewed bytes; a fresh plan then resolves the divergence.

## 3. Minimal production exposure

The Tauri `inspect_project_files` command currently calls the repository helper
directly. Add the same use case to `ProjectSyncService` so the external
integration test can use the production application interface with an
injected temporary repository.

Add this method to `impl ProjectSyncService` in
`src-tauri/src/project_sync/service.rs`:

```rust
pub fn inspect_project_files(
    &self,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectContentInventory, String> {
    inspect_project_files_with_repository(
        self.repository,
        local_project_id,
        storage_id,
    )
}
```

Change the Tauri command wrapper to call the service method:

```rust
#[tauri::command]
pub async fn inspect_project_files(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<ProjectContentInventory, String> {
    let repository = repository()?;
    run_blocking(move || {
        ProjectSyncService::new(&repository)
            .inspect_project_files(&local_project_id, &storage_id)
    })
    .await
}
```

Re-export the public project-content DTOs from `src-tauri/src/lib.rs`:

```rust
pub use project_sync::service::{
    // Existing exports remain here.
    ProjectContentEntry,
    ProjectContentEntryType,
    ProjectContentInventory,
};
```

Do not expose a test-only repository helper. `ProjectSyncService` is the
production interface and already owns the adjacent Push and Pull use cases.

## 4. Complex project fixture

Machine A uses this project tree:

```text
project-a/
├── .mallardignore                         included ignore policy
├── README.md                              included root file
├── docs/
│   ├── architecture/
│   │   ├── overview.md                    included file
│   │   └── decisions/
│   │       ├── 0001-storage.md            included file
│   │       └── 0002-review-first.md       included file
│   ├── guides/
│   │   └── onboarding.md                  included file
│   └── empty/                             included empty directory
├── src/
│   ├── app/
│   │   ├── main.ts                        included file
│   │   ├── debug.scratch                  ignored by *.scratch
│   │   └── config/
│   │       └── defaults.json              included file
│   └── shared/
│       └── format.ts                      included file
├── assets/
│   ├── fixtures/
│   │   └── payload.bin                    included binary file
│   └── empty/                             included empty directory
├── local-only/                            explicitly unselected directory
│   ├── notes.md                           explicitly unselected file
│   └── drafts/
│       └── experiment.txt                 explicitly unselected file
└── tmp/                                   ignored by tmp/
    └── cache/
        └── index.bin                      ignored file
```

The `.mallardignore` fixture contains:

```gitignore
tmp/
*.scratch
```

The fixture has four distinct classes:

| Class | Expected behavior |
|---|---|
| Included directories | Twelve directory resources are published, including two independent empty directories. |
| Included files | Ten file resources are published with exact hashes and sizes. |
| Explicitly unselected subtree | Four discovered resources remain local and become destination-specific exclusions only after successful Push. |
| Ignored content | The `tmp/` tree and `debug.scratch` never become inventory entries, recipe entries, manifest entries, or stored payloads. |

Machine B starts as an empty non-Git directory. Pull must reproduce only the
included class. It must not create the explicitly unselected or ignored
classes.

This fixture is complex enough to exercise independent branches, repeated
ancestor creation, deep paths, multiple file types, binary bytes, empty
directories, explicit selection, ignore rules, exact action counts, and stable
path-based identity. It deliberately excludes executables and
credential-shaped content. Those inputs require a separate warning-review
test and would make the basic round trip harder to diagnose.

## 5. Divergence scenarios

After the initial round trip, reuse four files from the reviewed tree. Machine
B's reviewed base is the first published revision. Machine A then publishes a
second revision while Machine B has independent local edits.

| Path | Storage after A's second Push | Local on B | Expected state on B | Pull decision |
|---|---|---|---|---|
| `docs/architecture/overview.md` | Changed from base | Changed differently from base | `diverged` | Show, approve, and preserve B's bytes as `overview-local-copy.md` before writing storage bytes. |
| `src/app/config/defaults.json` | Changed from base | Still equals base | `storage_ahead` | Show, approve, and fast-forward without a copy. |
| `src/shared/format.ts` | Still equals base | Changed from base | `local_ahead` | Do not show as a Pull candidate and do not modify it. |
| `README.md` | Still equals base | Still equals base | `synced` | Do not show as a Pull candidate and do not modify it. |

The test must derive the approved action IDs from exactly the paths whose
inventory state is `storage_ahead` or `diverged`. It must not approve all raw
project actions because the backend plan can contain settled project-resource
rows for receipt and baseline bookkeeping.

Before the first divergence Apply, change `overview.md` on Machine B again.
The pinned target digest must reject that stale action with `restore target
changed after planning`. The failed action must leave both the original path
and the proposed preservation-copy path unchanged. The independent
storage-ahead action may still settle because project-file actions are
isolated.

After a fresh Scan and Pull plan, approve the remaining divergent file. Apply
must then:

- copy Machine B's latest local bytes to
  `docs/architecture/overview-local-copy.md`;
- write Machine A's storage bytes to
  `docs/architecture/overview.md`;
- leave `src/shared/format.ts` at Machine B's local-ahead bytes;
- leave `README.md` unchanged; and
- advance the reviewed base only after the fresh plan settles.

The final Scan must report the two pulled originals as `synced`, the untouched
local edit as `local_ahead`, the preservation copy as `local_only`, and no
`storage_ahead` or `diverged` Pull candidate.

## 6. Complete integration-test source

Create `src-tauri/tests/project_files_e2e.rs` with this source after the
service exposure in section 3 is present:

```rust
//! Deterministic end-to-end coverage for reviewed project-file sync between
//! two non-Git project folders.
//!
//! The test uses two isolated Mallard homes, two isolated provider homes,
//! two different project roots, and one real local-folder object store. It
//! starts no provider process and never reads a real user home.

use mallard_domain::{
    sha256_hex, ActionId, ApplyPolicy, DraftProfileSelection, DraftRepositoryChoice,
    DraftStorageSelection, LogicalPath, ProjectFileSyncEligibilityState, ProjectSetupDraft,
    ProviderKey, RecipeEntry, RestoreAction, RestoreActionKind, RestorePlan, StorageConfig,
    StorageId, StorageKind, SyncScopeKey,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tauri_app_lib::{
    ProjectContentEntryType, ProjectContentInventory, ProjectDetail, ProjectSyncService,
    PushApproval, PushReviewContext,
};
use walkdir::WalkDir;

const INCLUDED_FILES: &[(&str, &[u8])] = &[
    (".mallardignore", b"tmp/\n*.scratch\n"),
    ("README.md", b"# Complex project fixture\n"),
    (
        "docs/architecture/overview.md",
        b"# Architecture\nReview before every mutation.\n",
    ),
    (
        "docs/architecture/decisions/0001-storage.md",
        b"# Decision 0001\nUse immutable storage objects.\n",
    ),
    (
        "docs/architecture/decisions/0002-review-first.md",
        b"# Decision 0002\nRequire explicit review.\n",
    ),
    (
        "docs/guides/onboarding.md",
        b"# Onboarding\nOpen, scan, review, then sync.\n",
    ),
    (
        "src/app/main.ts",
        b"export const applicationName = \"Mallard fixture\";\n",
    ),
    (
        "src/app/config/defaults.json",
        b"{\"reviewFirst\":true,\"retries\":2}\n",
    ),
    (
        "src/shared/format.ts",
        b"export const formatName = (value: string) => value.trim();\n",
    ),
    (
        "assets/fixtures/payload.bin",
        b"\x00\x01\x02\xffMALLARD-E2E\x00",
    ),
];

const INCLUDED_DIRECTORIES: &[&str] = &[
    "docs",
    "docs/architecture",
    "docs/architecture/decisions",
    "docs/guides",
    "docs/empty",
    "src",
    "src/app",
    "src/app/config",
    "src/shared",
    "assets",
    "assets/fixtures",
    "assets/empty",
];

const INCLUDED_PATHS: &[&str] = &[
    ".mallardignore",
    "README.md",
    "docs",
    "docs/architecture",
    "docs/architecture/overview.md",
    "docs/architecture/decisions",
    "docs/architecture/decisions/0001-storage.md",
    "docs/architecture/decisions/0002-review-first.md",
    "docs/guides",
    "docs/guides/onboarding.md",
    "docs/empty",
    "src",
    "src/app",
    "src/app/main.ts",
    "src/app/config",
    "src/app/config/defaults.json",
    "src/shared",
    "src/shared/format.ts",
    "assets",
    "assets/fixtures",
    "assets/fixtures/payload.bin",
    "assets/empty",
];

const EXCLUDED_FILES: &[(&str, &[u8])] = &[
    (
        "local-only/notes.md",
        b"Machine A notes must remain local.\n",
    ),
    (
        "local-only/drafts/experiment.txt",
        b"Unreviewed local experiment.\n",
    ),
];

const EXCLUDED_DIRECTORIES: &[&str] = &["local-only", "local-only/drafts"];
const EXCLUDED_PATHS: &[&str] = &[
    "local-only",
    "local-only/notes.md",
    "local-only/drafts",
    "local-only/drafts/experiment.txt",
];

const IGNORED_FILES: &[(&str, &[u8])] = &[
    ("tmp/cache/index.bin", b"IGNORED-TMP-PAYLOAD"),
    ("src/app/debug.scratch", b"IGNORED-SCRATCH-PAYLOAD"),
];
const IGNORED_PATHS: &[&str] = &[
    "tmp",
    "tmp/cache",
    "tmp/cache/index.bin",
    "src/app/debug.scratch",
];

const DIVERGED_PATH: &str = "docs/architecture/overview.md";
const DIVERGED_STORAGE_BYTES: &[u8] = b"# Architecture\nMachine A changed the storage version.\n";
const DIVERGED_LOCAL_AT_PLAN_BYTES: &[u8] =
    b"# Architecture\nMachine B changed the local version.\n";
const DIVERGED_LOCAL_AFTER_PLAN_BYTES: &[u8] =
    b"# Architecture\nMachine B changed it again after review.\n";
const DIVERGED_COPY_PATH: &str = "docs/architecture/overview-local-copy.md";

const STORAGE_AHEAD_PATH: &str = "src/app/config/defaults.json";
const STORAGE_AHEAD_BYTES: &[u8] = b"{\"reviewFirst\":true,\"retries\":3}\n";

const LOCAL_AHEAD_PATH: &str = "src/shared/format.ts";
const LOCAL_AHEAD_BYTES: &[u8] =
    b"export const formatName = (value: string) => `[local] ${value.trim()}`;\n";

const SYNCED_PATH: &str = "README.md";

fn codex_key() -> ProviderKey {
    ProviderKey::parse("codex").expect("static provider key")
}

fn write_fixture_file(project_root: &Path, relative_path: &str, bytes: &[u8]) {
    let path = project_root.join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture file parent");
    }
    std::fs::write(path, bytes).expect("write project fixture file");
}

fn create_complex_project_fixture(project_root: &Path) {
    for &relative_path in INCLUDED_DIRECTORIES
        .iter()
        .chain(EXCLUDED_DIRECTORIES.iter())
    {
        std::fs::create_dir_all(project_root.join(relative_path))
            .expect("create project fixture directory");
    }
    for &(relative_path, bytes) in INCLUDED_FILES
        .iter()
        .chain(EXCLUDED_FILES.iter())
        .chain(IGNORED_FILES.iter())
    {
        write_fixture_file(project_root, relative_path, bytes);
    }
}

fn logical_project_paths(relative_paths: &[&str]) -> BTreeSet<String> {
    relative_paths
        .iter()
        .map(|relative_path| format!("project/{relative_path}"))
        .collect()
}

fn included_file_bytes(relative_path: &str) -> &'static [u8] {
    INCLUDED_FILES
        .iter()
        .find_map(|(path, bytes)| (*path == relative_path).then_some(*bytes))
        .unwrap_or_else(|| panic!("fixture has no included file '{relative_path}'"))
}

fn local_storage(storage_root: &Path, storage_id: &str) -> StorageConfig {
    StorageConfig {
        id: StorageId::parse(storage_id).expect("static storage id"),
        name: "Project files E2E local storage".to_string(),
        kind: StorageKind::Local,
        bucket: String::new(),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        account_id: String::new(),
        s3_endpoint: String::new(),
        region: String::new(),
        local_dir: storage_root.to_string_lossy().into_owned(),
        supports_conditional_writes: None,
    }
}

fn finish_setup(
    service: &ProjectSyncService<'_>,
    project_root: &Path,
    provider_home: &Path,
    storage: StorageConfig,
    repository_choice: DraftRepositoryChoice,
) -> ProjectDetail {
    let mut draft: ProjectSetupDraft = service
        .create_setup_draft(project_root)
        .expect("create isolated project setup draft")
        .draft;
    draft.profiles.insert(
        codex_key(),
        DraftProfileSelection::Pending {
            path: provider_home.to_string_lossy().into_owned(),
            display_name: String::new(),
        },
    );
    draft.storage = Some(DraftStorageSelection::Pending { storage });
    draft.repository = repository_choice;

    let draft = service
        .update_setup_draft(draft)
        .expect("save isolated project setup choices");
    let inspection = service
        .inspect_setup_draft(&draft.draft_id)
        .expect("inspect isolated project setup");
    assert!(inspection.can_finalize, "project setup is not ready");

    service
        .finalize_project_setup(&draft.draft_id, draft.revision)
        .expect("finalize isolated project setup")
}

fn reviewed_project_file_approval(
    review: &PushReviewContext,
    inventory: &ProjectContentInventory,
    storage_id: &StorageId,
    selected_paths: &[&str],
) -> PushApproval {
    let selected_paths = selected_paths
        .iter()
        .map(|path| (*path).to_string())
        .collect::<BTreeSet<_>>();
    let mut approval = PushApproval::recommended(review, storage_id);

    // Project-file selection comes from the explicit Scan, not from the
    // provider inventory used to open Push. Remove every scanned project
    // resource before inserting the exact reviewed selection.
    for entry in &inventory.entries {
        approval
            .recipe
            .entries
            .remove(&entry.descriptor.resource_id);
    }

    let mut found = BTreeSet::new();
    for entry in &inventory.entries {
        if !selected_paths.contains(&entry.relative_path) {
            continue;
        }
        assert!(entry.local_present, "selected entry is not local");
        assert!(
            entry.blocked_reason.is_none(),
            "selected entry is blocked: {}",
            entry.relative_path,
        );
        assert_ne!(entry.entry_type, ProjectContentEntryType::Blocked);
        found.insert(entry.relative_path.clone());
        approval.recipe.entries.insert(
            entry.descriptor.resource_id.clone(),
            RecipeEntry {
                resource_id: entry.descriptor.resource_id.clone(),
                apply_policy: ApplyPolicy::ExplicitReview,
                required: entry.entry_type == ProjectContentEntryType::Directory,
            },
        );
    }

    assert_eq!(found, selected_paths, "Scan omitted a selected path");
    approval.project_content_review_token = Some(
        inventory
            .review_token
            .clone()
            .expect("eligible Scan has no review token"),
    );
    approval.project_content_removal_ids.clear();
    approval.acknowledged_warning_digests.clear();
    approval
}

fn project_action_ids(plan: &RestorePlan) -> Vec<ActionId> {
    plan.actions
        .iter()
        .filter(|action| {
            matches!(
                &action.kind,
                RestoreActionKind::WriteProjectFile { .. }
                    | RestoreActionKind::EnsureProjectDirectory { .. }
                    | RestoreActionKind::DeleteProjectFile { .. }
                    | RestoreActionKind::DeleteProjectDirectory { .. }
            )
        })
        .map(|action| action.action_id.clone())
        .collect()
}

fn project_write_action<'a>(plan: &'a RestorePlan, relative_path: &str) -> &'a RestoreAction {
    let expected_logical_path = format!("project/{relative_path}");
    plan.actions
        .iter()
        .find(|action| {
            matches!(
                &action.kind,
                RestoreActionKind::WriteProjectFile { logical_path, .. }
                    if logical_path.as_str() == expected_logical_path.as_str()
            )
        })
        .unwrap_or_else(|| panic!("Pull plan omitted project file '{relative_path}'"))
}

fn project_file_state<'a>(inventory: &'a ProjectContentInventory, relative_path: &str) -> &'a str {
    inventory
        .entries
        .iter()
        .find(|entry| entry.relative_path == relative_path)
        .unwrap_or_else(|| panic!("Scan omitted project file '{relative_path}'"))
        .state
        .as_str()
}

fn pull_candidate_paths(inventory: &ProjectContentInventory) -> BTreeSet<String> {
    inventory
        .entries
        .iter()
        .filter(|entry| matches!(entry.state.as_str(), "storage_ahead" | "diverged"))
        .map(|entry| entry.relative_path.clone())
        .collect()
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn assert_storage_omits(storage_root: &Path, needles: &[Vec<u8>]) {
    for entry in WalkDir::new(storage_root).follow_links(false) {
        let entry = entry.expect("walk isolated local storage");
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = std::fs::read(entry.path()).expect("read isolated storage object");
        for (index, needle) in needles.iter().enumerate() {
            assert!(
                !contains_bytes(&bytes, needle),
                "storage file '{}' contains private needle {}",
                entry.path().display(),
                index,
            );
        }
    }
}

#[test]
fn non_git_project_files_round_trip_through_reviewed_push_and_pull() {
    let temporary = tempfile::tempdir().expect("create project-files E2E root");
    let root = temporary.path();
    let machine_a_home = root.join("machine-a-home");
    let machine_b_home = root.join("machine-b-home");
    let provider_a = root.join("provider-a");
    let provider_b = root.join("provider-b");
    let project_a = root.join("project-a");
    let project_b = root.join("project-b");
    let storage_root = root.join("storage");

    for directory in [
        &machine_a_home,
        &machine_b_home,
        &provider_a,
        &provider_b,
        &project_a,
        &project_b,
        &storage_root,
    ] {
        std::fs::create_dir_all(directory).expect("create isolated E2E directory");
    }
    create_complex_project_fixture(&project_a);

    assert!(
        !project_a.join(".git").exists() && !project_b.join(".git").exists(),
        "E2E fixture unexpectedly became Git-managed",
    );

    let repository_a = LocalStateRepository::from_home_dir(&machine_a_home)
        .expect("open isolated Machine A state");
    let repository_b = LocalStateRepository::from_home_dir(&machine_b_home)
        .expect("open isolated Machine B state");
    let service_a = ProjectSyncService::new(&repository_a);
    let service_b = ProjectSyncService::new(&repository_b);
    let storage = local_storage(&storage_root, "project-files-e2e-round-trip");
    let storage_id = storage.id.clone();

    // Machine A creates a new remote repository. Finalize publishes only its
    // registration claim; ordinary project files still require Scan.
    let detail_a = finish_setup(
        &service_a,
        &project_a,
        &provider_a,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    assert!(detail_a.project.repository_fingerprint.is_none());
    assert!(detail_a.binding.is_some());
    let project_a_id = detail_a.project.local_project_id.clone();
    let bundle_id = detail_a.project.bundle_id.clone();

    let registration = service_a
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect registration-only bundle");
    assert!(registration.manifest.is_registration_only());

    let link_before_scan = repository_a
        .load_config()
        .expect("load Machine A config before Scan")
        .links
        .into_iter()
        .find(|link| link.local_project_id == project_a_id && link.storage_id == storage_id)
        .expect("Machine A project link before Scan");

    // This is the same order as the UI: open Push first, then explicitly
    // request Scan from the Project files step.
    let review = service_a
        .prepare_push_review(&project_a_id, &storage_id)
        .expect("prepare Machine A Push review");
    let after_review = service_a
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect bundle after opening Push");
    assert_eq!(after_review.head, registration.head);

    let scan_a = service_a
        .inspect_project_files(&project_a_id, &storage_id)
        .expect("scan Machine A project files");
    assert_eq!(
        scan_a.eligibility.state,
        ProjectFileSyncEligibilityState::Eligible,
    );
    assert!(scan_a.review_token.is_some());

    let entries_a = scan_a
        .entries
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(entries_a.len(), INCLUDED_PATHS.len() + EXCLUDED_PATHS.len(),);
    assert_eq!(scan_a.blocked_count, 0);
    for path in INCLUDED_PATHS.iter().chain(EXCLUDED_PATHS.iter()).copied() {
        let entry = entries_a
            .get(path)
            .unwrap_or_else(|| panic!("Scan omitted fixture entry '{path}'"));
        assert!(entry.newly_discovered);
        assert!(entry.selected_after_scan);
    }
    assert!(scan_a.ignored_count >= 2);
    for &ignored_path in IGNORED_PATHS {
        assert!(
            !entries_a.contains_key(ignored_path),
            "Scan exposed ignored path '{ignored_path}'",
        );
    }

    // Scan is read-only. It does not publish and does not persist the
    // tentative project-file selection.
    let after_scan = service_a
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect bundle after Scan");
    assert_eq!(after_scan.head, registration.head);
    let link_after_scan = repository_a
        .load_config()
        .expect("load Machine A config after Scan")
        .links
        .into_iter()
        .find(|link| link.local_project_id == project_a_id && link.storage_id == storage_id)
        .expect("Machine A project link after Scan");
    assert_eq!(link_after_scan.recipe, link_before_scan.recipe);
    assert_eq!(
        link_after_scan.project_content_preferences,
        link_before_scan.project_content_preferences,
    );

    let excluded_ids = EXCLUDED_PATHS
        .iter()
        .map(|path| entries_a[*path].descriptor.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let approval = reviewed_project_file_approval(&review, &scan_a, &storage_id, INCLUDED_PATHS);
    assert!(excluded_ids
        .iter()
        .all(|resource_id| !approval.recipe.entries.contains_key(resource_id)),);

    let pushed = service_a
        .push_reviewed(&project_a_id, &storage_id, approval)
        .expect("publish reviewed Machine A project files");
    assert!(pushed.success, "{}", pushed.message);
    assert!(pushed.revision.is_some());

    let published = service_a
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect published project files");
    assert_ne!(published.head, registration.head);
    let project_scope = published
        .manifest
        .scope(&SyncScopeKey::Project)
        .expect("published manifest has no project scope");
    assert_eq!(project_scope.files.len(), INCLUDED_FILES.len());
    assert_eq!(project_scope.directories.len(), INCLUDED_DIRECTORIES.len(),);

    for &(relative_path, expected_bytes) in INCLUDED_FILES {
        let logical_path = LogicalPath::parse(format!("project/{relative_path}"))
            .expect("valid fixture file path");
        let file = project_scope
            .files
            .get(&logical_path)
            .unwrap_or_else(|| panic!("published manifest omitted '{logical_path}'"));
        assert_eq!(file.sha256, sha256_hex(expected_bytes));
        assert_eq!(file.size, expected_bytes.len() as u64);
    }
    for &relative_path in INCLUDED_DIRECTORIES {
        let logical_path = LogicalPath::parse(format!("project/{relative_path}"))
            .expect("valid fixture directory path");
        assert!(
            project_scope.directories.contains_key(&logical_path),
            "published manifest omitted '{logical_path}'",
        );
    }
    for relative_path in EXCLUDED_PATHS.iter().chain(IGNORED_PATHS.iter()).copied() {
        let logical_path = LogicalPath::parse(format!("project/{relative_path}"))
            .expect("valid absent fixture path");
        assert!(
            !project_scope.files.contains_key(&logical_path)
                && !project_scope.directories.contains_key(&logical_path),
            "published manifest contains absent path '{logical_path}'",
        );
    }

    // A successful Push persists the exact unselected resources as local,
    // destination-specific exclusions. Those preferences do not travel.
    let link_after_push = repository_a
        .load_config()
        .expect("load Machine A config after Push")
        .links
        .into_iter()
        .find(|link| link.local_project_id == project_a_id && link.storage_id == storage_id)
        .expect("Machine A project link after Push");
    assert!(link_after_push
        .project_content_preferences
        .excluded_resource_ids
        .is_superset(&excluded_ids),);

    let manifest_json = serde_json::to_string(&published.manifest)
        .expect("serialize published manifest for privacy check");
    for private_path in [&machine_a_home, &provider_a, &project_a] {
        assert!(
            !manifest_json.contains(private_path.to_string_lossy().as_ref()),
            "manifest contains a Machine A absolute path",
        );
    }
    let mut storage_needles = vec![
        machine_a_home.to_string_lossy().as_bytes().to_vec(),
        provider_a.to_string_lossy().as_bytes().to_vec(),
        project_a.to_string_lossy().as_bytes().to_vec(),
    ];
    storage_needles.extend(
        EXCLUDED_FILES
            .iter()
            .chain(IGNORED_FILES.iter())
            .map(|(_, bytes)| bytes.to_vec()),
    );
    assert_storage_omits(&storage_root, &storage_needles);

    // Machine B connects a different non-Git folder to the existing bundle.
    let detail_b = finish_setup(
        &service_b,
        &project_b,
        &provider_b,
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: detail_a.project.display_name.clone(),
            repository_fingerprint: detail_a.project.repository_fingerprint.clone(),
            mismatch_acknowledged: false,
        },
    );
    assert!(detail_b.project.repository_fingerprint.is_none());
    assert_ne!(
        detail_b.project.local_project_id,
        detail_a.project.local_project_id,
    );
    assert_eq!(detail_b.project.bundle_id, bundle_id);
    let binding_b = detail_b.binding.expect("Machine B has no active binding");

    let plan = service_b
        .plan_bundle_restore(&storage_id, &bundle_id, &binding_b)
        .expect("plan Machine B Pull");
    assert_eq!(
        plan.project_content_eligibility
            .as_ref()
            .expect("Pull plan omitted project-file eligibility")
            .state,
        ProjectFileSyncEligibilityState::Eligible,
    );

    let approved_project_actions = project_action_ids(&plan);
    assert_eq!(
        approved_project_actions.len(),
        INCLUDED_DIRECTORIES.len() + INCLUDED_FILES.len(),
    );
    let canonical_project_b =
        std::fs::canonicalize(&project_b).expect("canonicalize Machine B project root");
    let mut planned_directories = BTreeSet::new();
    let mut planned_files = BTreeSet::new();
    for action in &plan.actions {
        match &action.kind {
            RestoreActionKind::EnsureProjectDirectory { logical_path, .. } => {
                assert!(action.requires_explicit_approval);
                planned_directories.insert(logical_path.as_str().to_string());
            }
            RestoreActionKind::WriteProjectFile { logical_path, .. } => {
                assert!(action.requires_explicit_approval);
                planned_files.insert(logical_path.as_str().to_string());
            }
            _ => continue,
        }
        let target = action
            .target_path
            .as_deref()
            .expect("project action has no target path");
        assert!(
            Path::new(target).starts_with(&canonical_project_b),
            "project action escaped Machine B's project root",
        );
    }
    assert_eq!(
        planned_directories,
        logical_project_paths(INCLUDED_DIRECTORIES)
    );
    assert_eq!(
        planned_files,
        INCLUDED_FILES
            .iter()
            .map(|(relative_path, _)| format!("project/{relative_path}"))
            .collect::<BTreeSet<_>>(),
    );
    assert!(INCLUDED_PATHS
        .iter()
        .all(|relative_path| !project_b.join(relative_path).exists()),);

    let applied = service_b
        .apply_bundle_restore(&plan.plan_id, approved_project_actions.clone())
        .expect("apply reviewed Machine B Pull");
    assert!(applied.success, "{}", applied.message);
    assert!(applied.failed_actions.is_empty());
    assert_eq!(applied.reviewed_base_advanced, Some(true));
    assert_eq!(
        applied
            .applied_action_ids
            .into_iter()
            .collect::<BTreeSet<_>>(),
        approved_project_actions
            .into_iter()
            .collect::<BTreeSet<_>>(),
    );

    for &(relative_path, expected_bytes) in INCLUDED_FILES {
        assert_eq!(
            std::fs::read(project_b.join(relative_path))
                .unwrap_or_else(|_| panic!("read restored Machine B file '{relative_path}'")),
            expected_bytes,
        );
        assert_eq!(
            std::fs::read(project_a.join(relative_path))
                .unwrap_or_else(|_| panic!("read unchanged Machine A file '{relative_path}'")),
            expected_bytes,
        );
    }
    for &relative_path in INCLUDED_DIRECTORIES {
        assert!(
            project_b.join(relative_path).is_dir(),
            "Machine B omitted directory '{relative_path}'",
        );
    }
    for relative_path in EXCLUDED_PATHS.iter().chain(IGNORED_PATHS.iter()).copied() {
        assert!(
            !project_b.join(relative_path).exists(),
            "Machine B received absent path '{relative_path}'",
        );
    }
    for &(relative_path, expected_bytes) in EXCLUDED_FILES.iter().chain(IGNORED_FILES.iter()) {
        assert_eq!(
            std::fs::read(project_a.join(relative_path))
                .unwrap_or_else(|_| panic!("read unchanged Machine A file '{relative_path}'")),
            expected_bytes,
        );
    }

    // Pull is now a fixed point. Preparing another review must not modify
    // storage, and the next Scan must have no Pull candidate.
    let _second_plan = service_b
        .plan_bundle_restore(&storage_id, &bundle_id, &binding_b)
        .expect("plan converged Machine B Pull");
    let after_pull = service_b
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect storage after Machine B Pull");
    assert_eq!(after_pull.head, published.head);

    let scan_b = service_b
        .inspect_project_files(&detail_b.project.local_project_id, &storage_id)
        .expect("scan restored Machine B project files");
    assert_eq!(
        scan_b.eligibility.state,
        ProjectFileSyncEligibilityState::Eligible,
    );
    assert_eq!(scan_b.entries.len(), INCLUDED_PATHS.len());
    assert_eq!(scan_b.blocked_count, 0);
    assert!(pull_candidate_paths(&scan_b).is_empty());
    let ids_a = scan_a
        .entries
        .iter()
        .filter(|entry| INCLUDED_PATHS.contains(&entry.relative_path.as_str()))
        .map(|entry| {
            (
                entry.relative_path.clone(),
                entry.descriptor.resource_id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let ids_b = scan_b
        .entries
        .iter()
        .filter(|entry| INCLUDED_PATHS.contains(&entry.relative_path.as_str()))
        .map(|entry| {
            assert_eq!(entry.state, "synced");
            assert!(entry.local_present);
            assert!(entry.storage_present);
            (
                entry.relative_path.clone(),
                entry.descriptor.resource_id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(ids_b, ids_a);
}

#[test]
fn divergent_project_file_pull_preserves_local_bytes_and_rejects_a_stale_plan() {
    let temporary = tempfile::tempdir().expect("create divergence E2E root");
    let root = temporary.path();
    let machine_a_home = root.join("machine-a-home");
    let machine_b_home = root.join("machine-b-home");
    let provider_a = root.join("provider-a");
    let provider_b = root.join("provider-b");
    let project_a = root.join("project-a");
    let project_b = root.join("project-b");
    let storage_root = root.join("storage");

    for directory in [
        &machine_a_home,
        &machine_b_home,
        &provider_a,
        &provider_b,
        &project_a,
        &project_b,
        &storage_root,
    ] {
        std::fs::create_dir_all(directory).expect("create isolated divergence directory");
    }
    create_complex_project_fixture(&project_a);

    let repository_a = LocalStateRepository::from_home_dir(&machine_a_home)
        .expect("open isolated Machine A state");
    let repository_b = LocalStateRepository::from_home_dir(&machine_b_home)
        .expect("open isolated Machine B state");
    let service_a = ProjectSyncService::new(&repository_a);
    let service_b = ProjectSyncService::new(&repository_b);
    let storage = local_storage(&storage_root, "project-files-e2e-divergence");
    let storage_id = storage.id.clone();

    // Establish revision 1 as the reviewed base on both machines.
    let detail_a = finish_setup(
        &service_a,
        &project_a,
        &provider_a,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let project_a_id = detail_a.project.local_project_id.clone();
    let bundle_id = detail_a.project.bundle_id.clone();
    let base_review = service_a
        .prepare_push_review(&project_a_id, &storage_id)
        .expect("prepare base Push review");
    let base_scan = service_a
        .inspect_project_files(&project_a_id, &storage_id)
        .expect("scan base project files");
    let base_push = service_a
        .push_reviewed(
            &project_a_id,
            &storage_id,
            reviewed_project_file_approval(&base_review, &base_scan, &storage_id, INCLUDED_PATHS),
        )
        .expect("publish reviewed base");
    assert!(base_push.success, "{}", base_push.message);

    let detail_b = finish_setup(
        &service_b,
        &project_b,
        &provider_b,
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: detail_a.project.display_name.clone(),
            repository_fingerprint: detail_a.project.repository_fingerprint.clone(),
            mismatch_acknowledged: false,
        },
    );
    let project_b_id = detail_b.project.local_project_id.clone();
    let binding_b = detail_b.binding.expect("Machine B has no active binding");
    let base_pull = service_b
        .plan_bundle_restore(&storage_id, &bundle_id, &binding_b)
        .expect("plan base Pull");
    let base_apply = service_b
        .apply_bundle_restore(&base_pull.plan_id, project_action_ids(&base_pull))
        .expect("apply base Pull");
    assert!(base_apply.success, "{}", base_apply.message);
    assert_eq!(base_apply.reviewed_base_advanced, Some(true));

    // Create all four comparison states against revision 1.
    write_fixture_file(&project_a, DIVERGED_PATH, DIVERGED_STORAGE_BYTES);
    write_fixture_file(&project_a, STORAGE_AHEAD_PATH, STORAGE_AHEAD_BYTES);
    write_fixture_file(&project_b, DIVERGED_PATH, DIVERGED_LOCAL_AT_PLAN_BYTES);
    write_fixture_file(&project_b, LOCAL_AHEAD_PATH, LOCAL_AHEAD_BYTES);

    let changed_review = service_a
        .prepare_push_review(&project_a_id, &storage_id)
        .expect("prepare changed Push review");
    let changed_scan = service_a
        .inspect_project_files(&project_a_id, &storage_id)
        .expect("scan changed Machine A project files");
    let changed_push = service_a
        .push_reviewed(
            &project_a_id,
            &storage_id,
            reviewed_project_file_approval(
                &changed_review,
                &changed_scan,
                &storage_id,
                INCLUDED_PATHS,
            ),
        )
        .expect("publish changed Machine A project files");
    assert!(changed_push.success, "{}", changed_push.message);
    let storage_after_changed_push = service_a
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect changed storage head");

    let divergent_scan = service_b
        .inspect_project_files(&project_b_id, &storage_id)
        .expect("scan Machine B divergence");
    assert_eq!(
        project_file_state(&divergent_scan, DIVERGED_PATH),
        "diverged"
    );
    assert_eq!(
        project_file_state(&divergent_scan, STORAGE_AHEAD_PATH),
        "storage_ahead"
    );
    assert_eq!(
        project_file_state(&divergent_scan, LOCAL_AHEAD_PATH),
        "local_ahead"
    );
    assert_eq!(project_file_state(&divergent_scan, SYNCED_PATH), "synced");
    let first_candidates = pull_candidate_paths(&divergent_scan);
    assert_eq!(
        first_candidates,
        BTreeSet::from([DIVERGED_PATH.to_string(), STORAGE_AHEAD_PATH.to_string(),])
    );

    let first_plan = service_b
        .plan_bundle_restore(&storage_id, &bundle_id, &binding_b)
        .expect("plan divergent Pull");
    let diverged_action = project_write_action(&first_plan, DIVERGED_PATH);
    let diverged_action_id = diverged_action.action_id.clone();
    assert_eq!(
        diverged_action.expected_target_sha256,
        Some(sha256_hex(DIVERGED_LOCAL_AT_PLAN_BYTES))
    );
    assert_eq!(
        diverged_action.source_sha256,
        Some(sha256_hex(DIVERGED_STORAGE_BYTES))
    );
    let diverged_pin = match &diverged_action.kind {
        RestoreActionKind::WriteProjectFile {
            preserve: Some(pin),
            ..
        } => pin,
        _ => panic!("diverged file has no preservation pin"),
    };
    assert_eq!(
        diverged_pin.copy_logical_path.as_str(),
        format!("project/{DIVERGED_COPY_PATH}")
    );
    assert_eq!(
        diverged_pin.local_content_sha256,
        sha256_hex(DIVERGED_LOCAL_AT_PLAN_BYTES)
    );
    assert_eq!(
        diverged_pin.storage_content_sha256,
        sha256_hex(DIVERGED_STORAGE_BYTES)
    );
    assert!(!diverged_pin.reusable_witness);

    let storage_ahead_action = project_write_action(&first_plan, STORAGE_AHEAD_PATH);
    let storage_ahead_action_id = storage_ahead_action.action_id.clone();
    assert!(matches!(
        &storage_ahead_action.kind,
        RestoreActionKind::WriteProjectFile { preserve: None, .. }
    ));

    let first_approved = first_candidates
        .iter()
        .map(|path| project_write_action(&first_plan, path).action_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        first_approved.iter().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([diverged_action_id.clone(), storage_ahead_action_id.clone(),])
    );

    // The local target changes after review. Its pinned action must fail,
    // while the independent storage-ahead action may settle.
    write_fixture_file(&project_b, DIVERGED_PATH, DIVERGED_LOCAL_AFTER_PLAN_BYTES);
    let stale_apply = service_b
        .apply_bundle_restore(&first_plan.plan_id, first_approved)
        .expect("apply stale divergent Pull");
    assert!(!stale_apply.success);
    assert_eq!(stale_apply.reviewed_base_advanced, Some(false));
    let stale_failure = stale_apply
        .failed_actions
        .iter()
        .find(|failure| failure.action_id == diverged_action_id)
        .expect("stale divergence did not fail");
    assert!(
        stale_failure
            .message
            .contains("restore target changed after planning"),
        "{}",
        stale_failure.message,
    );
    assert_eq!(
        std::fs::read(project_b.join(DIVERGED_PATH)).expect("read stale divergence target"),
        DIVERGED_LOCAL_AFTER_PLAN_BYTES,
    );
    assert!(!project_b.join(DIVERGED_COPY_PATH).exists());
    assert_eq!(
        std::fs::read(project_b.join(STORAGE_AHEAD_PATH))
            .expect("read independent fast-forward target"),
        STORAGE_AHEAD_BYTES,
    );
    assert_eq!(
        std::fs::read(project_b.join(LOCAL_AHEAD_PATH)).expect("read local-ahead target"),
        LOCAL_AHEAD_BYTES,
    );
    assert_eq!(
        std::fs::read(project_b.join(SYNCED_PATH)).expect("read synced target"),
        included_file_bytes(SYNCED_PATH),
    );

    // A fresh observation pins Machine B's latest bytes and resolves only
    // the remaining divergence.
    let refreshed_scan = service_b
        .inspect_project_files(&project_b_id, &storage_id)
        .expect("refresh divergent Machine B scan");
    assert_eq!(
        pull_candidate_paths(&refreshed_scan),
        BTreeSet::from([DIVERGED_PATH.to_string()])
    );
    assert_eq!(
        project_file_state(&refreshed_scan, STORAGE_AHEAD_PATH),
        "synced"
    );

    let fresh_plan = service_b
        .plan_bundle_restore(&storage_id, &bundle_id, &binding_b)
        .expect("replan divergent Pull");
    let fresh_action = project_write_action(&fresh_plan, DIVERGED_PATH);
    let fresh_pin = match &fresh_action.kind {
        RestoreActionKind::WriteProjectFile {
            preserve: Some(pin),
            ..
        } => pin,
        _ => panic!("fresh divergence has no preservation pin"),
    };
    assert_eq!(
        fresh_pin.local_content_sha256,
        sha256_hex(DIVERGED_LOCAL_AFTER_PLAN_BYTES)
    );
    let fresh_apply = service_b
        .apply_bundle_restore(&fresh_plan.plan_id, vec![fresh_action.action_id.clone()])
        .expect("apply fresh divergent Pull");
    assert!(fresh_apply.success, "{}", fresh_apply.message);
    assert!(fresh_apply.failed_actions.is_empty());
    assert_eq!(fresh_apply.reviewed_base_advanced, Some(true));

    assert_eq!(
        std::fs::read(project_b.join(DIVERGED_PATH)).expect("read resolved target"),
        DIVERGED_STORAGE_BYTES,
    );
    assert_eq!(
        std::fs::read(project_b.join(DIVERGED_COPY_PATH)).expect("read preserved local copy"),
        DIVERGED_LOCAL_AFTER_PLAN_BYTES,
    );
    assert_eq!(
        std::fs::read(project_b.join(STORAGE_AHEAD_PATH)).expect("read fast-forward target"),
        STORAGE_AHEAD_BYTES,
    );
    assert_eq!(
        std::fs::read(project_b.join(LOCAL_AHEAD_PATH)).expect("read local-ahead target"),
        LOCAL_AHEAD_BYTES,
    );
    assert_eq!(
        std::fs::read(project_b.join(SYNCED_PATH)).expect("read synced target"),
        included_file_bytes(SYNCED_PATH),
    );

    let final_scan = service_b
        .inspect_project_files(&project_b_id, &storage_id)
        .expect("scan resolved divergence");
    assert_eq!(project_file_state(&final_scan, DIVERGED_PATH), "synced");
    assert_eq!(
        project_file_state(&final_scan, STORAGE_AHEAD_PATH),
        "synced"
    );
    assert_eq!(
        project_file_state(&final_scan, LOCAL_AHEAD_PATH),
        "local_ahead"
    );
    assert_eq!(project_file_state(&final_scan, SYNCED_PATH), "synced");
    assert_eq!(
        project_file_state(&final_scan, DIVERGED_COPY_PATH),
        "local_only"
    );
    let preserved_entry = final_scan
        .entries
        .iter()
        .find(|entry| entry.relative_path == DIVERGED_COPY_PATH)
        .expect("final Scan omitted preservation copy");
    assert!(preserved_entry.newly_discovered);
    assert!(preserved_entry.selected_after_scan);
    assert!(pull_candidate_paths(&final_scan).is_empty());

    let storage_after_pulls = service_b
        .inspect_bundle(&storage_id, &bundle_id)
        .expect("inspect storage after divergent Pulls");
    assert_eq!(storage_after_pulls.head, storage_after_changed_push.head);
}
```

## 7. Why these tests use local-folder storage first

The local-folder implementation uses the real immutable object, manifest,
commit, and mutable-head model without a network dependency. It gives the
test direct access to the stored bytes for privacy assertions.

Do not mock the bundle engine in these tests. A mocked engine would stop the
tests before the publication and restore boundaries that they must prove.

Stub-S3 parity is a separate follow-up. It should reuse the same flow only
after these local-store tests are stable. Do not add a storage abstraction to
the first test binary solely for a future matrix.

## 8. Intentional exclusions

The E2E binary includes changed-content divergence and an apply-time file race.
It does not include remote deletion against a local edit, warning
acknowledgement, Git-after-review races, corrupt storage, preservation-name
exhaustion, or injected publication failure. Those cases have focused backend
coverage and should not make the main round trip or keep-both flow harder to
diagnose.

Add separate E2E tests later only for public-boundary gaps, starting with:

```text
changed_local_file_blocks_approved_remote_deletion
project_file_changed_after_scan_rejects_push
git_initialized_after_pull_plan_rejects_apply
```

## 9. Test command

Run the focused test with:

```sh
cargo test --manifest-path src-tauri/Cargo.toml \
  --test project_files_e2e
```

The normal backend command already uses `cargo test --workspace --all-targets`,
so it will include this integration test automatically:

```sh
npm run test:backend-integration
```

Optionally add this focused package script:

```json
"test:project-files-e2e": "cargo test --manifest-path src-tauri/Cargo.toml --test project_files_e2e"
```

## 10. Acceptance criteria

The implementation is complete when:

- both focused integration tests pass without an environment gate;
- it passes during normal parallel Cargo execution;
- it performs no global environment mutation;
- it uses no real `~/.mallard`, provider home, project, or storage;
- it starts no provider process and opens no network socket;
- it leaves no state after the temporary directory is dropped;
- it inspects the manifest and target filesystem directly;
- the divergent Pull candidate set contains only `storage_ahead` and
  `diverged` paths;
- the stale divergence action fails without overwriting either local path;
- the fresh divergence action creates the exact visible local copy before it
  writes the storage version;
- local-ahead and synced files remain byte-identical throughout Pull;
- `cargo fmt --all -- --check` passes;
- `cargo test --manifest-path src-tauri/Cargo.toml --test project_files_e2e`
  passes; and
- `npm run test:backend-integration` passes.
