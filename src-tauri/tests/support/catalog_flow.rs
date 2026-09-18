//! Reusable two-replica Library exercise shared by the deterministic and live
//! integration tests. Provider inventory, storage, and local
//! materialization use the production implementations. CLI execution is forbidden.
//!
//! The scope is the sync object: each replica links its scope to the shared
//! storage once, every push and pull is one review over rows, and all
//! resources of one coordinate converge into one repository.

use mallard_domain::{StorageConfig, StorageId};
use mallard_infrastructure::persistence::LocalStateRepository;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tauri_app_lib::{
    CatalogActionOutcome, CatalogCoordinate, CatalogLinkState, CatalogPullActionKind,
    CatalogPullApproval, CatalogPullDisposition, CatalogPushApproval, CatalogResourceId,
    CatalogResourceRelation, CatalogResourceRepositoryId, CatalogScopeId, CatalogScopePullPlan,
    CatalogScopePushPlan, CmdOutput, PluginProcessRunner, ProjectSyncService, ProviderKey,
    StorageCatalogService, CATALOG_SCHEMA,
};
use walkdir::WalkDir;

const ALPHA: &str = "mallard-alpha";
const BETA: &str = "mallard-beta";
const ALPHA_COPY: &str = "mallard-alpha-b-local-copy";
const BETA_COPY: &str = "mallard-beta-a-local-copy";
const DUCK: &str = "mallard-duck-v1";
const OWL: &str = "mallard-owl-v2";
const DUCK_COPY: &str = "mallard-duck-b-local-copy";
const OWL_COPY: &str = "mallard-owl-a-local-copy";

fn codex() -> ProviderKey {
    ProviderKey::parse("codex").expect("valid provider key")
}

fn skills_coordinate() -> CatalogCoordinate {
    CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1)
        .expect("valid skills coordinate")
}

fn pets_coordinate(version: u32) -> CatalogCoordinate {
    CatalogCoordinate::new("codex", "desktop_pet", "openai.codex-pet", version)
        .expect("valid pets coordinate")
}

pub struct CatalogFlowInputs<'a> {
    pub machine_home_a: &'a Path,
    pub machine_home_b: &'a Path,
    pub user_home_a: &'a Path,
    pub user_home_b: &'a Path,
    pub codex_home_a: &'a Path,
    pub codex_home_b: &'a Path,
    pub storage: &'a StorageConfig,
    pub privacy_needles: &'a [Vec<u8>],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogFlowReport {
    /// One repository per coordinate: skills, pets v1, pets v2.
    pub repository_count: usize,
    /// Final head revisions of the coordinate repositories, sorted ascending.
    pub kind_repository_revisions: Vec<u64>,
    pub native_provider_process_calls: usize,
}

#[derive(Clone)]
struct ScopeSet {
    skills: CatalogScopeId,
    pets_v1: CatalogScopeId,
    pets_v2: CatalogScopeId,
}

struct Replica {
    service: StorageCatalogService,
    machine_home: PathBuf,
    codex_home: PathBuf,
    scopes: ScopeSet,
}

fn recommended_push_approval(plan: &CatalogScopePushPlan) -> CatalogPushApproval {
    let selected_resource_ids = plan
        .rows
        .iter()
        .filter(|row| row.row.selected_by_default)
        .map(|row| row.row.resource_id.clone())
        .collect::<Vec<_>>();
    let acknowledged_warning_digests = plan
        .rows
        .iter()
        .filter(|row| row.row.selected_by_default)
        .flat_map(|row| row.row.warnings.iter())
        .map(|warning| warning.digest.clone())
        .collect();
    CatalogPushApproval {
        plan_id: plan.plan_id.clone(),
        selected_resource_ids,
        acknowledged_warning_digests,
    }
}

fn recommended_pull_approval(plan: &CatalogScopePullPlan) -> CatalogPullApproval {
    CatalogPullApproval {
        plan_id: plan.plan_id.clone(),
        choices: plan
            .items
            .iter()
            .filter_map(|item| {
                item.item
                    .default_action_id
                    .clone()
                    .map(|action_id| (item.item.resource_id.clone(), action_id))
            })
            .collect(),
        destructive_acknowledged: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PullMutation {
    Install,
    Update,
    Preserve,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileSnapshot {
    bytes: Vec<u8>,
    mode: u32,
}

type TreeSnapshot = BTreeMap<String, FileSnapshot>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExactTreeEntrySnapshot {
    Directory {
        mode: u32,
        modified: Option<SystemTime>,
    },
    File {
        bytes: Vec<u8>,
        mode: u32,
        modified: Option<SystemTime>,
    },
}

type ExactTreeSnapshot = BTreeMap<String, ExactTreeEntrySnapshot>;

struct RejectProviderRunner {
    calls: Arc<AtomicUsize>,
}

impl PluginProcessRunner for RejectProviderRunner {
    fn run(
        &self,
        provider: &ProviderKey,
        home: &Path,
        argv: &[String],
    ) -> Result<CmdOutput, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        panic!(
            "Library file flow invoked {} at {}: {}",
            provider.as_str(),
            home.display(),
            argv.join(" ")
        );
    }
}

fn add_storage(machine_home: &Path, storage: &StorageConfig) {
    let repository =
        LocalStateRepository::from_home_dir(machine_home).expect("open isolated Mallard state");
    repository
        .mutate_config(|config| {
            match config
                .storages
                .iter()
                .find(|candidate| candidate.id == storage.id)
            {
                Some(existing) if existing != storage => {
                    return Err("test storage id is already configured differently".to_string())
                }
                Some(_) => return Ok(()),
                None => config.storages.push(storage.clone()),
            }
            Ok(())
        })
        .expect("register shared local-folder storage");
}

fn create_replica(
    machine_home: &Path,
    user_home: &Path,
    codex_home: &Path,
    storage: &StorageConfig,
    calls: Arc<AtomicUsize>,
) -> Replica {
    for directory in [machine_home, user_home, codex_home] {
        fs::create_dir_all(directory).expect("create isolated Library home");
    }
    add_storage(machine_home, storage);
    let canonical_codex_home = fs::canonicalize(codex_home).expect("canonicalize Codex home");
    let service = StorageCatalogService::from_home_dirs(
        machine_home.to_path_buf(),
        user_home.to_path_buf(),
        Box::new(RejectProviderRunner { calls }),
    )
    .expect("open isolated Library service");
    let profile = service
        .save_profile(codex(), canonical_codex_home.to_string_lossy().to_string())
        .expect("save isolated Codex profile");
    let create_scope = |coordinate: CatalogCoordinate| {
        service
            .create_scope(
                coordinate.provider,
                profile.profile_id.clone(),
                coordinate.resource_kind,
                coordinate.format,
                coordinate.format_version,
            )
            .expect("create Library scope")
            .scope
            .scope_id
    };
    let scopes = ScopeSet {
        skills: create_scope(skills_coordinate()),
        pets_v1: create_scope(pets_coordinate(1)),
        pets_v2: create_scope(pets_coordinate(2)),
    };
    Replica {
        service,
        machine_home: machine_home.to_path_buf(),
        codex_home: canonical_codex_home,
        scopes,
    }
}

fn skill_path(replica: &Replica, name: &str) -> PathBuf {
    replica.codex_home.join("skills").join(name)
}

fn pet_path(replica: &Replica, name: &str) -> PathBuf {
    replica.codex_home.join("pets").join(name)
}

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set fixture mode");
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) {
    fs::create_dir_all(path.parent().expect("fixture file has parent"))
        .expect("create fixture parent");
    fs::write(path, bytes).expect("write fixture file");
    set_mode(path, mode);
}

fn write_skill(replica: &Replica, name: &str, version: &str) -> TreeSnapshot {
    let root = skill_path(replica, name);
    fs::create_dir_all(&root).expect("create skill directory");
    write_file(
        &root.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: Mallard integration fixture\n---\n\nSkill body {version}.\n"
        )
        .as_bytes(),
        0o644,
    );
    write_file(
        &root.join("references/contract.md"),
        format!("# Contract\n\nNested reference {version}.\n").as_bytes(),
        0o644,
    );
    write_file(
        &root.join("examples/input.txt"),
        format!("input={version}\n").as_bytes(),
        0o644,
    );
    write_file(
        &root.join("scripts/check.sh"),
        format!("#!/bin/sh\n# {version}\nexit 0\n").as_bytes(),
        0o755,
    );
    snapshot_tree(&root)
}

fn webp_atlas(version: u32, marker: u8) -> Vec<u8> {
    let (width, height) = if version == 2 {
        (1536, 2288)
    } else {
        (1536, 1872)
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&20_u32.to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    bytes.extend_from_slice(b"VP8L");
    bytes.extend_from_slice(&12_u32.to_le_bytes());
    bytes.push(0x2F);
    let width_minus_one = width - 1;
    let height_minus_one = height - 1;
    let packed: u32 = (width_minus_one & 0x3FFF) | ((height_minus_one & 0x3FFF) << 14);
    bytes.extend_from_slice(&packed.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, marker]);
    bytes
}

fn pet_manifest(
    version: u32,
    pet_id: &str,
    display_name: &str,
    description: &str,
    atlas_basename: &str,
) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct Manifest<'a> {
        id: &'a str,
        #[serde(rename = "displayName")]
        display_name: &'a str,
        description: &'a str,
        #[serde(
            rename = "spriteVersionNumber",
            skip_serializing_if = "Option::is_none"
        )]
        sprite_version_number: Option<u32>,
        #[serde(rename = "spritesheetPath")]
        spritesheet_path: &'a str,
    }
    let manifest = Manifest {
        id: pet_id,
        display_name,
        description,
        sprite_version_number: (version == 2).then_some(2),
        spritesheet_path: atlas_basename,
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest).expect("serialize fake pet manifest");
    bytes.push(b'\n');
    bytes
}

#[allow(clippy::too_many_arguments)]
fn write_pet(
    replica: &Replica,
    version: u32,
    pet_id: &str,
    display_name: &str,
    description: &str,
    atlas_basename: &str,
    marker: u8,
) -> TreeSnapshot {
    let root = pet_path(replica, pet_id);
    fs::create_dir_all(&root).expect("create pet directory");
    write_file(
        &root.join("pet.json"),
        &pet_manifest(version, pet_id, display_name, description, atlas_basename),
        0o644,
    );
    write_file(
        &root.join(atlas_basename),
        &webp_atlas(version, marker),
        0o644,
    );
    snapshot_tree(&root)
}

fn portable_mode(path: &Path, bytes: &[u8]) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::symlink_metadata(path)
            .expect("inspect fixture file")
            .permissions()
            .mode();
        if mode & 0o111 != 0 || bytes.starts_with(b"#!") {
            return 0o755;
        }
    }
    if bytes.starts_with(b"#!") {
        0o755
    } else {
        0o644
    }
}

fn snapshot_tree(root: &Path) -> TreeSnapshot {
    let mut snapshot = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
        let entry = entry.expect("walk synchronized resource tree");
        let metadata = entry
            .metadata()
            .expect("inspect synchronized resource entry");
        assert!(
            !entry.file_type().is_symlink(),
            "test resource contains a symlink"
        );
        if metadata.is_dir() {
            continue;
        }
        assert!(metadata.is_file(), "test resource contains a special file");
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("resource entry stays below root")
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(entry.path()).expect("read synchronized resource file");
        let mode = portable_mode(entry.path(), &bytes);
        assert!(snapshot
            .insert(relative, FileSnapshot { bytes, mode })
            .is_none());
    }
    snapshot
}

fn exact_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

fn snapshot_tree_exact(root: &Path) -> ExactTreeSnapshot {
    let root_metadata = fs::symlink_metadata(root).expect("inspect synchronized resource root");
    assert!(root_metadata.is_dir());
    assert!(!root_metadata.file_type().is_symlink());
    let mut snapshot = BTreeMap::from([(
        ".".to_string(),
        ExactTreeEntrySnapshot::Directory {
            mode: exact_mode(&root_metadata),
            modified: root_metadata.modified().ok(),
        },
    )]);
    for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
        let entry = entry.expect("walk exact synchronized resource tree");
        let metadata = entry
            .metadata()
            .expect("inspect exact synchronized resource entry");
        assert!(
            !entry.file_type().is_symlink(),
            "test resource contains a symlink"
        );
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("resource entry stays below root")
            .to_string_lossy()
            .replace('\\', "/");
        let value = if metadata.is_dir() {
            ExactTreeEntrySnapshot::Directory {
                mode: exact_mode(&metadata),
                modified: metadata.modified().ok(),
            }
        } else {
            assert!(metadata.is_file(), "test resource contains a special file");
            ExactTreeEntrySnapshot::File {
                bytes: fs::read(entry.path()).expect("read exact synchronized resource file"),
                mode: exact_mode(&metadata),
                modified: metadata.modified().ok(),
            }
        };
        assert!(snapshot.insert(relative, value).is_none());
    }
    snapshot
}

fn renamed_skill_snapshot(source: &TreeSnapshot, copy_name: &str) -> TreeSnapshot {
    let mut renamed = source.clone();
    let skill_md = renamed
        .get_mut("SKILL.md")
        .expect("skill fixture has SKILL.md");
    let text = String::from_utf8(skill_md.bytes.clone()).expect("skill declaration is UTF-8");
    let mut output = String::new();
    let mut replaced = false;
    for line in text.split_inclusive('\n') {
        if !replaced && line.trim_start().starts_with("name:") {
            let indent = &line[..line.len() - line.trim_start().len()];
            output.push_str(indent);
            output.push_str("name: ");
            output.push_str(copy_name);
            output.push('\n');
            replaced = true;
        } else {
            output.push_str(line);
        }
    }
    assert!(replaced, "skill fixture declares a name");
    skill_md.bytes = output.into_bytes();
    renamed
}

fn renamed_pet_snapshot(
    source: &TreeSnapshot,
    version: u32,
    copy_name: &str,
    description: &str,
    atlas_basename: &str,
) -> TreeSnapshot {
    let mut renamed = source.clone();
    renamed
        .get_mut("pet.json")
        .expect("pet fixture has manifest")
        .bytes = pet_manifest(version, copy_name, copy_name, description, atlas_basename);
    renamed
}

fn assert_tree(path: &Path, expected: &TreeSnapshot) {
    assert_eq!(
        snapshot_tree(path),
        *expected,
        "tree mismatch at {}",
        path.display()
    );
}

fn assert_disk_mode(path: &Path, expected: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let actual = fs::symlink_metadata(path)
            .expect("inspect synchronized file mode")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            actual,
            expected,
            "unexpected filesystem mode at {}",
            path.display()
        );
    }
    #[cfg(not(unix))]
    let _ = (path, expected);
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

pub fn storage_snapshot(root: &Path) -> BTreeMap<String, (u64, [u8; 32])> {
    let mut result = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
        let entry = entry.expect("walk local-folder storage");
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = fs::read(entry.path()).expect("read local-folder storage object");
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("storage object stays below root")
            .to_string_lossy()
            .replace('\\', "/");
        assert!(result
            .insert(relative, (bytes.len() as u64, Sha256::digest(bytes).into()))
            .is_none());
    }
    result
}

fn link_scope(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    expected_completion: &str,
) {
    let outcome = replica
        .service
        .link_scope_storage(scope_id, storage_id)
        .expect("link Library scope to storage");
    assert_eq!(outcome.completion, expected_completion);
}

fn assert_row_state(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    name: &str,
    expected: CatalogLinkState,
) {
    let page = replica
        .service
        .list_scope_resources(scope_id, Some(storage_id))
        .expect("list Library scope page");
    let row = page
        .rows
        .iter()
        .find(|row| row.display_name == name)
        .unwrap_or_else(|| panic!("row '{name}' is absent from its Library scope page"));
    assert_eq!(row.state, Some(expected), "unexpected state for '{name}'");
}

fn push_resource(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    name: &str,
    expected_relation: CatalogResourceRelation,
    expected_revision: u64,
) -> CatalogResourceId {
    let plan = replica
        .service
        .prepare_push(scope_id, storage_id)
        .expect("prepare reviewed Library Push");
    let row = plan
        .rows
        .iter()
        .find(|row| row.row.display_name == name)
        .unwrap_or_else(|| panic!("push review lacks a row for '{name}'"));
    assert_eq!(row.row.relation, expected_relation);
    assert!(row.row.selectable);
    assert!(row.row.selected_by_default);
    assert!(!row.row.held_for_pull);
    let resource_id = row.row.resource_id.clone();
    let mut approval = CatalogPushApproval {
        plan_id: plan.plan_id.clone(),
        selected_resource_ids: vec![resource_id.clone()],
        acknowledged_warning_digests: Vec::new(),
    };
    if row.row.coordinate == skills_coordinate() {
        let executable = row
            .row
            .warnings
            .iter()
            .find(|warning| warning.code == "executable_content")
            .expect("executable skill fixture did not produce its reviewed warning");
        assert!(
            executable.message.contains("scripts/check.sh"),
            "executable warning did not identify the reviewed fixture path"
        );
    }
    if !row.row.warnings.is_empty() {
        let error = replica
            .service
            .apply_push(approval.clone())
            .expect_err("unacknowledged resource warning must block publication");
        assert!(
            error.contains("unacknowledged warning"),
            "unexpected warning acknowledgement error: {error}"
        );
        approval.acknowledged_warning_digests = row
            .row
            .warnings
            .iter()
            .map(|warning| warning.digest.clone())
            .collect();
        // The discarded first attempt consumed nothing; the plan survives a
        // refused apply only if apply fails before consuming it.
        let _ = &approval;
    }
    let outcome = replica
        .service
        .apply_push(approval)
        .expect("publish reviewed Library resources");
    assert!(outcome.published, "{}", outcome.message);
    assert_eq!(outcome.revision, Some(expected_revision));
    resource_id
}

fn assert_pull_kind(kind: &CatalogPullActionKind, expected: PullMutation) {
    assert!(
        matches!(
            (kind, expected),
            (CatalogPullActionKind::Install, PullMutation::Install)
                | (CatalogPullActionKind::Update, PullMutation::Update)
                | (
                    CatalogPullActionKind::PreserveAsCopyAndOverwrite { .. },
                    PullMutation::Preserve
                )
        ),
        "unexpected Pull action: {kind:?}"
    );
}

fn prepare_pull(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    requested_copy_names: BTreeMap<CatalogResourceId, String>,
) -> CatalogScopePullPlan {
    replica
        .service
        .prepare_pull(scope_id, storage_id, requested_copy_names)
        .expect("prepare reviewed Library Pull")
}

#[allow(clippy::too_many_arguments)]
fn pull_resource(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    name: &str,
    expected_relation: CatalogResourceRelation,
    expected_mutation: PullMutation,
    copy_name: Option<&str>,
    expected_outcome: CatalogActionOutcome,
) {
    let mut plan = prepare_pull(replica, scope_id, storage_id, BTreeMap::new());
    let find_item = |plan: &CatalogScopePullPlan| -> CatalogResourceId {
        plan.items
            .iter()
            .find(|item| item.item.display_name == name)
            .unwrap_or_else(|| panic!("pull review lacks an item for '{name}'"))
            .item
            .resource_id
            .clone()
    };
    let resource_id = find_item(&plan);
    if let Some(copy_name) = copy_name {
        // Choosing a copy name re-prepares the review, exactly as the UI does.
        plan = prepare_pull(
            replica,
            scope_id,
            storage_id,
            BTreeMap::from([(resource_id.clone(), copy_name.to_string())]),
        );
    }
    let item = plan
        .items
        .iter()
        .find(|item| item.item.resource_id == resource_id)
        .expect("re-prepared pull review keeps its item");
    assert_eq!(item.item.relation, expected_relation);
    assert_eq!(item.item.disposition, CatalogPullDisposition::Executable);
    let action_id = item
        .item
        .default_action_id
        .clone()
        .expect("mutating Pull has a recommended action");
    let action = item
        .item
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .expect("recommended Pull action exists");
    assert_pull_kind(&action.kind, expected_mutation);
    if let Some(copy_name) = copy_name {
        let preservation = action
            .kind
            .preservation()
            .expect("divergent Pull defaults to preservation");
        assert_eq!(preservation.copy_name, copy_name);
        assert!(!action.requires_destructive_approval);
        let destructive = item
            .item
            .actions
            .iter()
            .find(|candidate| candidate.kind.is_destructive_overwrite())
            .expect("divergent Pull review lacks its explicit overwrite alternative");
        assert!(destructive.requires_destructive_approval);
        let error = replica
            .service
            .apply_pull(CatalogPullApproval {
                plan_id: plan.plan_id.clone(),
                choices: BTreeMap::from([(resource_id.clone(), destructive.action_id.clone())]),
                destructive_acknowledged: Vec::new(),
            })
            .expect_err("destructive overwrite without acknowledgement must fail");
        assert!(
            error.contains("requires explicit acknowledgement"),
            "unexpected destructive acknowledgement error: {error}"
        );
    }
    let outcome = replica
        .service
        .apply_pull(CatalogPullApproval {
            plan_id: plan.plan_id.clone(),
            choices: BTreeMap::from([(resource_id.clone(), action_id)]),
            destructive_acknowledged: Vec::new(),
        })
        .expect("apply reviewed Library Pull");
    assert!(outcome.remote_accepted);
    let result = outcome
        .results
        .iter()
        .find(|result| result.resource_id == resource_id)
        .expect("pull outcome reports the approved row");
    assert_eq!(result.outcome, expected_outcome);
}

fn assert_stale_push_rejected(replica: &Replica, stale_plan: CatalogScopePushPlan, name: &str) {
    let row = stale_plan
        .rows
        .iter()
        .find(|row| row.row.display_name == name)
        .expect("stale push review carries its row");
    let approval = CatalogPushApproval {
        plan_id: stale_plan.plan_id.clone(),
        selected_resource_ids: vec![row.row.resource_id.clone()],
        acknowledged_warning_digests: row
            .row
            .warnings
            .iter()
            .map(|warning| warning.digest.clone())
            .collect(),
    };
    let error = replica
        .service
        .apply_push(approval)
        .expect_err("stale reviewed Push must fail");
    assert!(
        error.contains("storage head changed after review"),
        "unexpected stale Push error: {error}"
    );
}

fn assert_divergent_push_held(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    name: &str,
) {
    let plan = replica
        .service
        .prepare_push(scope_id, storage_id)
        .expect("prepare divergent Library Push review");
    let row = plan
        .rows
        .iter()
        .find(|row| row.row.display_name == name)
        .expect("divergent push review carries its row");
    assert_eq!(row.row.relation, CatalogResourceRelation::ChangedBoth);
    assert!(!row.row.selectable);
    assert!(!row.row.selected_by_default);
    assert!(row.row.held_for_pull);
    let error = replica
        .service
        .apply_push(CatalogPushApproval {
            plan_id: plan.plan_id.clone(),
            selected_resource_ids: vec![row.row.resource_id.clone()],
            acknowledged_warning_digests: row
                .row
                .warnings
                .iter()
                .map(|warning| warning.digest.clone())
                .collect(),
        })
        .expect_err("divergent Push cannot bypass Pull review");
    assert!(error.contains("Pull") || error.contains("safe to Push"));
}

struct KindFlow {
    repository_id: CatalogResourceRepositoryId,
    revision: u64,
    resource_ids: BTreeSet<CatalogResourceId>,
}

fn skill_inventory(a: &Replica, b: &Replica, names: &[&str]) -> BTreeMap<String, TreeSnapshot> {
    let mut inventory = BTreeMap::new();
    for (replica_name, replica) in [("a", a), ("b", b)] {
        for name in names {
            inventory.insert(
                format!("{replica_name}/skills/{name}"),
                snapshot_tree(&skill_path(replica, name)),
            );
        }
    }
    inventory
}

fn clear_replica_storage(replica: &Replica, storage_id: &StorageId, expect_remote_content: bool) {
    let repository = LocalStateRepository::from_home_dir(&replica.machine_home)
        .expect("open replica state for storage clear");
    let expected_config_revision = repository
        .load_config()
        .expect("load storage-clear config")
        .revision;
    let result = ProjectSyncService::new(&repository)
        .clear_storage_data(storage_id, expected_config_revision)
        .expect("clear storage through the production service");

    assert_eq!(result.repository_objects_deleted, 0);
    assert_eq!(result.catalog_links_reset, 1);
    assert_eq!(result.project_links_reset, 0);
    if expect_remote_content {
        assert!(result.library_objects_deleted > 0);
    } else {
        assert_eq!(result.library_objects_deleted, 0);
    }
}

fn reset_and_republish_skills(
    a: &Replica,
    b: &Replica,
    storage_id: &StorageId,
    expected_resource_ids: &BTreeSet<CatalogResourceId>,
) -> u64 {
    let local_before_reset = skill_inventory(a, b, &[ALPHA, BETA]);

    clear_replica_storage(a, storage_id, true);
    clear_replica_storage(b, storage_id, false);

    for replica in [a, b] {
        assert!(replica
            .service
            .list_storage_repositories(storage_id)
            .expect("list repositories after storage clear")
            .is_empty());
    }
    assert_eq!(
        skill_inventory(a, b, &[ALPHA, BETA]),
        local_before_reset,
        "storage clear changed a local skill tree"
    );

    let push = a
        .service
        .prepare_push(&a.scopes.skills, storage_id)
        .expect("prepare skill republish after storage clear");
    assert_eq!(
        push.rows
            .iter()
            .map(|row| row.row.resource_id.clone())
            .collect::<BTreeSet<_>>(),
        *expected_resource_ids
    );
    for row in &push.rows {
        assert_eq!(row.row.relation, CatalogResourceRelation::LocalOnly);
        assert!(row.row.selectable);
        assert!(row.row.selected_by_default);
        assert!(!row.row.held_for_pull);
    }
    let outcome = a
        .service
        .apply_push(recommended_push_approval(&push))
        .expect("republish skills after storage clear");
    assert!(outcome.published);
    assert_eq!(outcome.revision, Some(1));

    let pull = prepare_pull(b, &b.scopes.skills, storage_id, BTreeMap::new());
    assert_eq!(
        pull.items
            .iter()
            .map(|item| item.item.resource_id.clone())
            .collect::<BTreeSet<_>>(),
        *expected_resource_ids
    );
    for item in &pull.items {
        assert_eq!(item.item.relation, CatalogResourceRelation::Matching);
        assert_eq!(
            item.item.disposition,
            CatalogPullDisposition::AlreadyMatching
        );
        assert!(item.item.actions.is_empty());
        assert!(item.item.default_action_id.is_none());
    }
    let outcome = b
        .service
        .apply_pull(recommended_pull_approval(&pull))
        .expect("accept republished matching skills");
    assert!(outcome.remote_accepted);
    assert!(outcome
        .results
        .iter()
        .all(|result| result.outcome == CatalogActionOutcome::AlreadyMatching));
    assert_eq!(
        skill_inventory(a, b, &[ALPHA, BETA]),
        local_before_reset,
        "skill republish changed a matching local tree"
    );

    1
}

#[allow(clippy::too_many_lines)]
fn run_skill_flow(
    a: &Replica,
    b: &Replica,
    storage_id: &StorageId,
    reset_storage_mid_flow: bool,
) -> KindFlow {
    let mut revision = 0_u64;
    let alpha_v1 = write_skill(a, ALPHA, "alpha-v1-from-a");
    let beta_v1 = write_skill(b, BETA, "beta-v1-from-b");

    // A links first: the storage is empty, nothing to review.
    link_scope(a, &a.scopes.skills, storage_id, "open");
    revision += 1;
    let alpha_id = push_resource(
        a,
        &a.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::LocalOnly,
        revision,
    );

    // B links second: the storage already holds alpha, but B has no local
    // copy, so the row is LocalMissing and the link routes to Pull.
    link_scope(b, &b.scopes.skills, storage_id, "review_pull");
    assert_row_state(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA,
        CatalogLinkState::LocalMissing,
    );
    pull_resource(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&skill_path(b, ALPHA), &alpha_v1);

    revision += 1;
    let beta_id = push_resource(
        b,
        &b.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::LocalOnly,
        revision,
    );
    pull_resource(
        a,
        &a.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&skill_path(a, BETA), &beta_v1);

    // Ordinary next revisions travel in both directions.
    let alpha_v2 = write_skill(a, ALPHA, "alpha-v2-from-a");
    assert_row_state(
        a,
        &a.scopes.skills,
        storage_id,
        ALPHA,
        CatalogLinkState::LocalChanged,
    );
    revision += 1;
    push_resource(
        a,
        &a.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::LocalChanged,
        revision,
    );
    assert_row_state(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA,
        CatalogLinkState::StorageChanged,
    );
    pull_resource(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(b, ALPHA), &alpha_v2);

    let beta_v2 = write_skill(b, BETA, "beta-v2-from-b");
    revision += 1;
    push_resource(
        b,
        &b.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::LocalChanged,
        revision,
    );
    pull_resource(
        a,
        &a.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(a, BETA), &beta_v2);

    if reset_storage_mid_flow {
        revision = reset_and_republish_skills(
            a,
            b,
            storage_id,
            &BTreeSet::from([alpha_id.clone(), beta_id.clone()]),
        );
    }

    // Alpha diverges. A wins the storage race; B's local branch is preserved.
    let alpha_a_winner = write_skill(a, ALPHA, "alpha-v3-a-storage-winner");
    let alpha_b_loser = write_skill(b, ALPHA, "alpha-v3-b-preserved-local");
    let alpha_b_stale = b
        .service
        .prepare_push(&b.scopes.skills, storage_id)
        .expect("prepare alpha losing Push");
    revision += 1;
    push_resource(
        a,
        &a.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::LocalChanged,
        revision,
    );
    assert_stale_push_rejected(b, alpha_b_stale, ALPHA);
    assert_divergent_push_held(b, &b.scopes.skills, storage_id, ALPHA);
    pull_resource(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::ChangedBoth,
        PullMutation::Preserve,
        Some(ALPHA_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&skill_path(b, ALPHA), &alpha_a_winner);
    let alpha_copy_snapshot = renamed_skill_snapshot(&alpha_b_loser, ALPHA_COPY);
    assert_tree(&skill_path(b, ALPHA_COPY), &alpha_copy_snapshot);

    // The preserved copy is an ordinary new resource in the same repository.
    revision += 1;
    let alpha_copy_id = push_resource(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA_COPY,
        CatalogResourceRelation::LocalOnly,
        revision,
    );
    assert_ne!(alpha_copy_id, alpha_id);
    pull_resource(
        a,
        &a.scopes.skills,
        storage_id,
        ALPHA_COPY,
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&skill_path(a, ALPHA_COPY), &alpha_copy_snapshot);

    // Beta diverges in the opposite direction. B wins; A is preserved.
    let beta_a_loser = write_skill(a, BETA, "beta-v3-a-preserved-local");
    let beta_b_winner = write_skill(b, BETA, "beta-v3-b-storage-winner");
    let beta_a_stale = a
        .service
        .prepare_push(&a.scopes.skills, storage_id)
        .expect("prepare beta losing Push");
    revision += 1;
    push_resource(
        b,
        &b.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::LocalChanged,
        revision,
    );
    assert_stale_push_rejected(a, beta_a_stale, BETA);
    assert_divergent_push_held(a, &a.scopes.skills, storage_id, BETA);
    pull_resource(
        a,
        &a.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::ChangedBoth,
        PullMutation::Preserve,
        Some(BETA_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&skill_path(a, BETA), &beta_b_winner);
    let beta_copy_snapshot = renamed_skill_snapshot(&beta_a_loser, BETA_COPY);
    assert_tree(&skill_path(a, BETA_COPY), &beta_copy_snapshot);

    revision += 1;
    let beta_copy_id = push_resource(
        a,
        &a.scopes.skills,
        storage_id,
        BETA_COPY,
        CatalogResourceRelation::LocalOnly,
        revision,
    );
    assert_ne!(beta_copy_id, beta_id);
    pull_resource(
        b,
        &b.scopes.skills,
        storage_id,
        BETA_COPY,
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&skill_path(b, BETA_COPY), &beta_copy_snapshot);

    // A preserved conflict does not poison the resource: the former losing
    // replica can publish a normal next revision in either direction.
    let alpha_v4 = write_skill(b, ALPHA, "alpha-v4-from-former-loser-b");
    revision += 1;
    push_resource(
        b,
        &b.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::LocalChanged,
        revision,
    );
    pull_resource(
        a,
        &a.scopes.skills,
        storage_id,
        ALPHA,
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(a, ALPHA), &alpha_v4);

    let beta_v4 = write_skill(a, BETA, "beta-v4-from-former-loser-a");
    revision += 1;
    push_resource(
        a,
        &a.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::LocalChanged,
        revision,
    );
    pull_resource(
        b,
        &b.scopes.skills,
        storage_id,
        BETA,
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(b, BETA), &beta_v4);

    KindFlow {
        repository_id: CatalogResourceRepositoryId::for_coordinate(&skills_coordinate()),
        revision,
        resource_ids: BTreeSet::from([alpha_id, beta_id, alpha_copy_id, beta_copy_id]),
    }
}

#[allow(clippy::too_many_lines)]
fn run_pet_flow(a: &Replica, b: &Replica, storage_id: &StorageId) -> (KindFlow, KindFlow) {
    let mut v1_revision = 0_u64;
    let mut v2_revision = 0_u64;
    let duck_v1 = write_pet(a, 1, DUCK, "Mallard Duck", "duck-v1-from-a", "duck.webp", 1);
    let owl_v1 = write_pet(b, 2, OWL, "Mallard Owl", "owl-v1-from-b", "owl.webp", 11);

    link_scope(a, &a.scopes.pets_v1, storage_id, "open");
    v1_revision += 1;
    let duck_id = push_resource(
        a,
        &a.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::LocalOnly,
        v1_revision,
    );
    link_scope(b, &b.scopes.pets_v1, storage_id, "review_pull");
    pull_resource(
        b,
        &b.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&pet_path(b, DUCK), &duck_v1);

    link_scope(b, &b.scopes.pets_v2, storage_id, "open");
    v2_revision += 1;
    let owl_id = push_resource(
        b,
        &b.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::LocalOnly,
        v2_revision,
    );
    link_scope(a, &a.scopes.pets_v2, storage_id, "review_pull");
    pull_resource(
        a,
        &a.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_ne!(duck_id, owl_id);
    assert_tree(&pet_path(a, OWL), &owl_v1);

    // The receiving replica may publish the next ordinary revision.
    let duck_v2 = write_pet(
        b,
        1,
        DUCK,
        "Mallard Duck",
        "duck-v2-from-receiver-b",
        "duck.webp",
        2,
    );
    v1_revision += 1;
    push_resource(
        b,
        &b.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::LocalChanged,
        v1_revision,
    );
    pull_resource(
        a,
        &a.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(a, DUCK), &duck_v2);

    let owl_v2 = write_pet(
        a,
        2,
        OWL,
        "Mallard Owl",
        "owl-v2-from-receiver-a",
        "owl.webp",
        12,
    );
    v2_revision += 1;
    push_resource(
        a,
        &a.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::LocalChanged,
        v2_revision,
    );
    pull_resource(
        b,
        &b.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(b, OWL), &owl_v2);

    // Duck divergence: A wins the remote head and B preserves its local pet.
    let duck_a_winner = write_pet(
        a,
        1,
        DUCK,
        "Mallard Duck",
        "duck-v3-a-storage-winner",
        "duck.webp",
        3,
    );
    let duck_b_loser = write_pet(
        b,
        1,
        DUCK,
        "Mallard Duck",
        "duck-v3-b-preserved-local",
        "duck.webp",
        4,
    );
    let duck_b_stale = b
        .service
        .prepare_push(&b.scopes.pets_v1, storage_id)
        .expect("prepare duck losing Push");
    v1_revision += 1;
    push_resource(
        a,
        &a.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::LocalChanged,
        v1_revision,
    );
    assert_stale_push_rejected(b, duck_b_stale, "Mallard Duck");
    assert_divergent_push_held(b, &b.scopes.pets_v1, storage_id, "Mallard Duck");
    pull_resource(
        b,
        &b.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::ChangedBoth,
        PullMutation::Preserve,
        Some(DUCK_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&pet_path(b, DUCK), &duck_a_winner);
    let duck_copy_snapshot = renamed_pet_snapshot(
        &duck_b_loser,
        1,
        DUCK_COPY,
        "duck-v3-b-preserved-local",
        "duck.webp",
    );
    assert_tree(&pet_path(b, DUCK_COPY), &duck_copy_snapshot);

    v1_revision += 1;
    let duck_copy_id = push_resource(
        b,
        &b.scopes.pets_v1,
        storage_id,
        DUCK_COPY,
        CatalogResourceRelation::LocalOnly,
        v1_revision,
    );
    assert_ne!(duck_copy_id, duck_id);
    pull_resource(
        a,
        &a.scopes.pets_v1,
        storage_id,
        DUCK_COPY,
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&pet_path(a, DUCK_COPY), &duck_copy_snapshot);

    // Owl divergence reverses the winner/loser roles and preserves v2.
    let owl_a_loser = write_pet(
        a,
        2,
        OWL,
        "Mallard Owl",
        "owl-v3-a-preserved-local",
        "owl.webp",
        13,
    );
    let owl_b_winner = write_pet(
        b,
        2,
        OWL,
        "Mallard Owl",
        "owl-v3-b-storage-winner",
        "owl.webp",
        14,
    );
    let owl_a_stale = a
        .service
        .prepare_push(&a.scopes.pets_v2, storage_id)
        .expect("prepare owl losing Push");
    v2_revision += 1;
    push_resource(
        b,
        &b.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::LocalChanged,
        v2_revision,
    );
    assert_stale_push_rejected(a, owl_a_stale, "Mallard Owl");
    assert_divergent_push_held(a, &a.scopes.pets_v2, storage_id, "Mallard Owl");
    pull_resource(
        a,
        &a.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::ChangedBoth,
        PullMutation::Preserve,
        Some(OWL_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&pet_path(a, OWL), &owl_b_winner);
    let owl_copy_snapshot = renamed_pet_snapshot(
        &owl_a_loser,
        2,
        OWL_COPY,
        "owl-v3-a-preserved-local",
        "owl.webp",
    );
    assert_tree(&pet_path(a, OWL_COPY), &owl_copy_snapshot);

    v2_revision += 1;
    let owl_copy_id = push_resource(
        a,
        &a.scopes.pets_v2,
        storage_id,
        OWL_COPY,
        CatalogResourceRelation::LocalOnly,
        v2_revision,
    );
    assert_ne!(owl_copy_id, owl_id);
    pull_resource(
        b,
        &b.scopes.pets_v2,
        storage_id,
        OWL_COPY,
        CatalogResourceRelation::StorageOnly,
        PullMutation::Install,
        None,
        CatalogActionOutcome::Installed,
    );
    assert_tree(&pet_path(b, OWL_COPY), &owl_copy_snapshot);

    let duck_v4 = write_pet(
        b,
        1,
        DUCK,
        "Mallard Duck",
        "duck-v4-from-former-loser-b",
        "duck.webp",
        5,
    );
    v1_revision += 1;
    push_resource(
        b,
        &b.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::LocalChanged,
        v1_revision,
    );
    pull_resource(
        a,
        &a.scopes.pets_v1,
        storage_id,
        "Mallard Duck",
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(a, DUCK), &duck_v4);

    let owl_v4 = write_pet(
        a,
        2,
        OWL,
        "Mallard Owl",
        "owl-v4-from-former-loser-a",
        "owl.webp",
        15,
    );
    v2_revision += 1;
    push_resource(
        a,
        &a.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::LocalChanged,
        v2_revision,
    );
    pull_resource(
        b,
        &b.scopes.pets_v2,
        storage_id,
        "Mallard Owl",
        CatalogResourceRelation::StorageChanged,
        PullMutation::Update,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(b, OWL), &owl_v4);

    (
        KindFlow {
            repository_id: CatalogResourceRepositoryId::for_coordinate(&pets_coordinate(1)),
            revision: v1_revision,
            resource_ids: BTreeSet::from([duck_id, duck_copy_id]),
        },
        KindFlow {
            repository_id: CatalogResourceRepositoryId::for_coordinate(&pets_coordinate(2)),
            revision: v2_revision,
            resource_ids: BTreeSet::from([owl_id, owl_copy_id]),
        },
    )
}

fn assert_noop_fixed_point(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    expected_resource_ids: &BTreeSet<CatalogResourceId>,
) {
    let push = replica
        .service
        .prepare_push(scope_id, storage_id)
        .expect("prepare fixed-point Push");
    assert_eq!(
        push.rows
            .iter()
            .map(|row| row.row.resource_id.clone())
            .collect::<BTreeSet<_>>(),
        *expected_resource_ids,
        "kind repository holds exactly the converged resource set"
    );
    for row in &push.rows {
        assert_eq!(row.row.relation, CatalogResourceRelation::Matching);
        assert!(!row.row.selectable);
        assert!(!row.row.selected_by_default);
        assert!(!row.row.held_for_pull);
    }
    let outcome = replica
        .service
        .apply_push(recommended_push_approval(&push))
        .expect("skip fixed-point Push");
    assert!(!outcome.published);

    let pull = prepare_pull(replica, scope_id, storage_id, BTreeMap::new());
    for item in &pull.items {
        assert_eq!(item.item.relation, CatalogResourceRelation::Matching);
        assert_eq!(
            item.item.disposition,
            CatalogPullDisposition::AlreadyMatching
        );
        assert!(item.item.actions.is_empty());
        assert!(item.item.default_action_id.is_none());
    }
    let outcome = replica
        .service
        .apply_pull(recommended_pull_approval(&pull))
        .expect("apply fixed-point Pull");
    assert!(outcome.remote_accepted);
    assert!(outcome
        .results
        .iter()
        .all(|result| result.outcome == CatalogActionOutcome::AlreadyMatching));

    let after = replica
        .service
        .prepare_push(scope_id, storage_id)
        .expect("reinspect fixed-point link after no-op Pull");
    assert_eq!(after.link_revision, push.link_revision);
    assert!(after.rows.iter().all(|row| !row.row.selectable));
}

fn assert_final_local_trees(a: &Replica, b: &Replica) {
    for name in [ALPHA, BETA, ALPHA_COPY, BETA_COPY] {
        let left = snapshot_tree(&skill_path(a, name));
        let right = snapshot_tree(&skill_path(b, name));
        assert_eq!(left, right, "final skill differs between replicas: {name}");
        assert_eq!(left["SKILL.md"].mode, 0o644);
        assert_eq!(left["references/contract.md"].mode, 0o644);
        assert_eq!(left["examples/input.txt"].mode, 0o644);
        assert_eq!(left["scripts/check.sh"].mode, 0o755);
        for replica in [a, b] {
            let root = skill_path(replica, name);
            assert_disk_mode(&root.join("SKILL.md"), 0o644);
            assert_disk_mode(&root.join("references/contract.md"), 0o644);
            assert_disk_mode(&root.join("examples/input.txt"), 0o644);
            assert_disk_mode(&root.join("scripts/check.sh"), 0o755);
        }
    }
    for name in [DUCK, OWL, DUCK_COPY, OWL_COPY] {
        let left = snapshot_tree(&pet_path(a, name));
        let right = snapshot_tree(&pet_path(b, name));
        assert_eq!(left, right, "final pet differs between replicas: {name}");
        assert_eq!(
            left.len(),
            2,
            "pet package must contain only manifest and atlas"
        );
        assert!(
            left.values().all(|file| file.mode == 0o644),
            "pet manifest and atlas must use portable regular-file modes"
        );
        for replica in [a, b] {
            for relative in left.keys() {
                assert_disk_mode(&pet_path(replica, name).join(relative), 0o644);
            }
        }
    }
    assert!(!fs::read_to_string(pet_path(a, DUCK).join("pet.json"))
        .expect("read v1 pet manifest")
        .contains("spriteVersionNumber"));
    assert!(fs::read_to_string(pet_path(a, OWL).join("pet.json"))
        .expect("read v2 pet manifest")
        .contains("\"spriteVersionNumber\": 2"));
}

fn final_local_tree_inventory(a: &Replica, b: &Replica) -> BTreeMap<String, ExactTreeSnapshot> {
    let mut inventory = BTreeMap::new();
    for (replica_name, replica) in [("a", a), ("b", b)] {
        for name in [ALPHA, BETA, ALPHA_COPY, BETA_COPY] {
            inventory.insert(
                format!("{replica_name}/skills/{name}"),
                snapshot_tree_exact(&skill_path(replica, name)),
            );
        }
        for name in [DUCK, OWL, DUCK_COPY, OWL_COPY] {
            inventory.insert(
                format!("{replica_name}/pets/{name}"),
                snapshot_tree_exact(&pet_path(replica, name)),
            );
        }
    }
    inventory
}

fn assert_storage_privacy(storage_root: &Path, needles: &[Vec<u8>]) -> usize {
    assert_eq!(
        CATALOG_SCHEMA, 1,
        "Library schema must remain at the clean-cutover value"
    );
    let library_root = storage_root.join(".mallard/v1/library");
    assert!(library_root.is_dir(), "Library namespace was not created");
    assert!(
        !storage_root.join(".mallard/v1/catalog").exists(),
        "legacy flat catalog namespace must not reappear"
    );
    let mut repositories = BTreeSet::new();
    for entry in fs::read_dir(&library_root).expect("list Library repositories") {
        let entry = entry.expect("read Library repository entry");
        if entry.file_type().expect("inspect Library entry").is_dir() {
            repositories.insert(entry.file_name());
        }
    }
    assert_eq!(
        repositories.len(),
        3,
        "one converged repository per kind: skills, pets v1, pets v2"
    );

    // The shared store may also contain project-bundle payloads. Raw provider
    // rollouts legitimately carry their captured cwd, so the Library-specific
    // path scan must stay inside the independent Library namespace. The live
    // harness applies its separate project-bundle privacy contract to the
    // rest of the store.
    for entry in WalkDir::new(&library_root).follow_links(false).min_depth(1) {
        let entry = entry.expect("walk portable Library storage");
        assert!(
            !entry.file_type().is_symlink(),
            "portable storage contains a symlink"
        );
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = fs::read(entry.path()).expect("read portable Library object");
        for needle in needles {
            assert!(
                !contains_bytes(&bytes, needle),
                "portable Library object '{}' leaked a private input",
                entry.path().display()
            );
        }
        let relative = entry
            .path()
            .strip_prefix(&library_root)
            .expect("portable Library object remains below its namespace");
        let relative_bytes = relative.to_string_lossy();
        for needle in needles {
            assert!(
                !contains_bytes(relative_bytes.as_bytes(), needle),
                "portable Library object key leaked a private input"
            );
        }
        assert!(
            !relative_bytes.contains("auth.json"),
            "authentication file name reached portable storage"
        );
    }
    repositories.len()
}

/// Exercise real Library Push/Pull review, immutable publication, CAS,
/// preservation, materialization, and fixed-point behavior for two isolated
/// Codex homes. It is safe to call after a conversation flow because the
/// Library namespace is independent of project bundle heads.
#[allow(dead_code)]
pub fn run_catalog_flow(inputs: CatalogFlowInputs<'_>) -> CatalogFlowReport {
    run_catalog_flow_inner(inputs, false)
}

/// Run the deterministic catalog flow with a whole-storage clear between the
/// ordinary update phase and the divergence phase. Use only with a dedicated
/// storage because the clear also removes project repositories by design.
#[allow(dead_code)]
pub fn run_catalog_flow_with_storage_reset(inputs: CatalogFlowInputs<'_>) -> CatalogFlowReport {
    run_catalog_flow_inner(inputs, true)
}

fn run_catalog_flow_inner(
    inputs: CatalogFlowInputs<'_>,
    reset_storage_mid_flow: bool,
) -> CatalogFlowReport {
    for (label, path) in [
        ("machine A home", inputs.machine_home_a),
        ("machine B home", inputs.machine_home_b),
        ("user A home", inputs.user_home_a),
        ("user B home", inputs.user_home_b),
        ("Codex A home", inputs.codex_home_a),
        ("Codex B home", inputs.codex_home_b),
    ] {
        assert!(path.is_absolute(), "{label} must be absolute");
        fs::create_dir_all(path).unwrap_or_else(|error| panic!("create {label}: {error}"));
    }
    inputs
        .storage
        .validate()
        .expect("valid shared test storage");
    let storage_root = mallard_application::storage_setup::storage_local_root(
        mallard_infrastructure::storage_providers::StorageProviderRegistry::production(),
        inputs.storage,
    )
    .expect("resolve the shared test storage")
    .expect("the shared test storage is a local folder");
    let storage_root = storage_root.as_path();
    assert!(storage_root.is_absolute());
    fs::create_dir_all(storage_root).expect("create shared local-folder storage");

    let calls = Arc::new(AtomicUsize::new(0));
    let a = create_replica(
        inputs.machine_home_a,
        inputs.user_home_a,
        inputs.codex_home_a,
        inputs.storage,
        calls.clone(),
    );
    let b = create_replica(
        inputs.machine_home_b,
        inputs.user_home_b,
        inputs.codex_home_b,
        inputs.storage,
        calls.clone(),
    );

    let skills = run_skill_flow(&a, &b, &inputs.storage.id, reset_storage_mid_flow);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "skills flow invoked a provider CLI"
    );
    let (pets_v1, pets_v2) = run_pet_flow(&a, &b, &inputs.storage.id);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "pets flow invoked a provider CLI"
    );
    assert_eq!(skills.resource_ids.len(), 4);
    assert_eq!(pets_v1.resource_ids.len(), 2);
    assert_eq!(pets_v2.resource_ids.len(), 2);

    assert_final_local_trees(&a, &b);
    let local_before_noops = final_local_tree_inventory(&a, &b);
    let before_noops = storage_snapshot(storage_root);
    // The storage page's listing sees exactly the three converged kind
    // repositories. Each row has passed the production engine's tag, head,
    // and manifest validation before its facts are exposed here.
    for replica in [&a, &b] {
        let listed = replica
            .service
            .list_storage_repositories(&inputs.storage.id)
            .expect("list Catalog repositories in the storage");
        let facts: BTreeMap<String, (Option<u64>, usize)> = listed
            .iter()
            .map(|repo| {
                assert!(repo.error.is_none(), "unreadable row: {:?}", repo.error);
                (
                    repo.repository_id.clone(),
                    (repo.revision, repo.resource_names.len()),
                )
            })
            .collect();
        let expected: BTreeMap<String, (Option<u64>, usize)> = [&skills, &pets_v1, &pets_v2]
            .into_iter()
            .map(|flow| {
                (
                    flow.repository_id.as_str().to_string(),
                    (Some(flow.revision), flow.resource_ids.len()),
                )
            })
            .collect();
        assert_eq!(facts, expected);
    }
    for replica in [&a, &b] {
        for (scope_id, flow) in [
            (&replica.scopes.skills, &skills),
            (&replica.scopes.pets_v1, &pets_v1),
            (&replica.scopes.pets_v2, &pets_v2),
        ] {
            assert_noop_fixed_point(replica, scope_id, &inputs.storage.id, &flow.resource_ids);
        }
    }
    assert_eq!(
        storage_snapshot(storage_root),
        before_noops,
        "fixed-point Push/Pull created or replaced a portable object"
    );
    assert_eq!(
        final_local_tree_inventory(&a, &b),
        local_before_noops,
        "fixed-point Push/Pull changed a local skill or pet tree"
    );
    let repository_count = assert_storage_privacy(storage_root, inputs.privacy_needles);

    let mut kind_repository_revisions = vec![skills.revision, pets_v1.revision, pets_v2.revision];
    kind_repository_revisions.sort_unstable();
    CatalogFlowReport {
        repository_count,
        kind_repository_revisions,
        native_provider_process_calls: calls.load(Ordering::Relaxed),
    }
}
