//! Reusable two-replica Library exercise shared by the deterministic and live
//! integration tests. All provider discovery is injected; storage and local
//! materialization use the production implementations.

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
    CatalogActionOutcome, CatalogLinkId, CatalogLinkState, CatalogPublishApproval,
    CatalogPullActionKind, CatalogPullApproval, CatalogPullDisposition, CatalogRegistrationId,
    CatalogResource, CatalogResourceId, CatalogResourceKindTag, CatalogResourceRelation,
    CatalogResourceRepositoryId, CatalogScopeId, CmdOutput, CodexPetFormat,
    LinkCatalogScopeResourceRequest, PluginProcessRunner, ProviderKey, StorageCatalogService,
    CATALOG_SCHEMA,
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
    pub repository_count: usize,
    pub primary_revisions: Vec<u64>,
    pub preserved_copy_revisions: Vec<u64>,
    pub native_provider_process_calls: usize,
    pub injected_inventory_calls: usize,
}

#[derive(Clone)]
struct ScopeSet {
    skills: CatalogScopeId,
    pets_v1: CatalogScopeId,
    pets_v2: CatalogScopeId,
}

struct Replica {
    service: StorageCatalogService,
    codex_home: PathBuf,
    scopes: ScopeSet,
}

#[derive(Clone)]
struct SyncedResource {
    a_link: CatalogLinkId,
    b_link: CatalogLinkId,
    repository_id: CatalogResourceRepositoryId,
    resource_id: CatalogResourceId,
}

#[derive(Clone)]
struct LinkedResource {
    link_id: CatalogLinkId,
    registration_id: CatalogRegistrationId,
    repository_id: CatalogResourceRepositoryId,
    resource_id: CatalogResourceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PullMutation {
    InstallSkill,
    UpdateSkill,
    PreserveSkill,
    InstallPet,
    UpdatePet,
    PreservePet,
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

struct EmptyCodexRunner {
    expected_home: PathBuf,
    calls: Arc<AtomicUsize>,
}

impl PluginProcessRunner for EmptyCodexRunner {
    fn run(
        &self,
        provider: ProviderKey,
        home: &Path,
        argv: &[String],
    ) -> Result<CmdOutput, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if provider != ProviderKey::Codex {
            return Err(format!(
                "unexpected provider process: {}",
                provider.as_str()
            ));
        }
        if home != self.expected_home {
            return Err("provider discovery escaped its selected Codex home".to_string());
        }
        let stdout = match argv
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["plugin", "marketplace", "list", "--json"] => r#"{"marketplaces":[]}"#,
            ["plugin", "list", "--json"] => r#"{"installed":[]}"#,
            _ => return Err(format!("unexpected provider command: {}", argv.join(" "))),
        };
        Ok(CmdOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
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
    label: &str,
) -> Replica {
    for directory in [machine_home, user_home, codex_home] {
        fs::create_dir_all(directory).expect("create isolated Library home");
    }
    add_storage(machine_home, storage);
    let canonical_codex_home = fs::canonicalize(codex_home).expect("canonicalize Codex home");
    let service = StorageCatalogService::from_home_dirs(
        machine_home.to_path_buf(),
        user_home.to_path_buf(),
        Box::new(EmptyCodexRunner {
            expected_home: canonical_codex_home.clone(),
            calls,
        }),
    )
    .expect("open isolated Library service");
    let profile = service
        .save_profile(
            ProviderKey::Codex,
            format!("Codex {label}"),
            canonical_codex_home.to_string_lossy().to_string(),
        )
        .expect("save isolated Codex profile");
    let create_scope = |kind| {
        service
            .create_scope(ProviderKey::Codex, profile.profile_id.clone(), kind)
            .expect("create Library scope")
            .scope
            .scope_id
    };
    let scopes = ScopeSet {
        skills: create_scope(CatalogResourceKindTag::AgentSkill),
        pets_v1: create_scope(CatalogResourceKindTag::CodexPetV1),
        pets_v2: create_scope(CatalogResourceKindTag::CodexPetV2),
    };
    Replica {
        service,
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

fn webp_atlas(format: CodexPetFormat, marker: u8) -> Vec<u8> {
    let (width, height) = match format {
        CodexPetFormat::V1 => (1536, 1872),
        CodexPetFormat::V2 => (1536, 2288),
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
    format: CodexPetFormat,
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
        sprite_version_number: (format == CodexPetFormat::V2).then_some(2),
        spritesheet_path: atlas_basename,
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest).expect("serialize fake pet manifest");
    bytes.push(b'\n');
    bytes
}

fn write_pet(
    replica: &Replica,
    format: CodexPetFormat,
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
        &pet_manifest(format, pet_id, display_name, description, atlas_basename),
        0o644,
    );
    write_file(
        &root.join(atlas_basename),
        &webp_atlas(format, marker),
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
    format: CodexPetFormat,
    copy_name: &str,
    description: &str,
    atlas_basename: &str,
) -> TreeSnapshot {
    let mut renamed = source.clone();
    renamed
        .get_mut("pet.json")
        .expect("pet fixture has manifest")
        .bytes = pet_manifest(format, copy_name, copy_name, description, atlas_basename);
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

fn link_local_resource(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    local_directory: &Path,
) -> LinkedResource {
    let page = replica
        .service
        .list_scope_resources(scope_id, Some(storage_id))
        .expect("list local Library resource");
    let local_directory = fs::canonicalize(local_directory).expect("canonicalize local resource");
    let row = page
        .rows
        .iter()
        .find(|row| {
            row.local_present
                && !row.linked
                && row
                    .local_path
                    .as_ref()
                    .and_then(|path| fs::canonicalize(path).ok())
                    .as_ref()
                    == Some(&local_directory)
        })
        .unwrap_or_else(|| {
            panic!(
                "local resource '{}' is absent from its Library scope",
                local_directory.display()
            )
        });
    let local_resource_id = row
        .resource_id
        .clone()
        .expect("local Library row has a resource id");
    let outcome = replica
        .service
        .link_scope_resource(LinkCatalogScopeResourceRequest {
            scope_id: scope_id.clone(),
            storage_id: storage_id.clone(),
            local_resource_id: Some(local_resource_id.clone()),
            repository_id: None,
        })
        .expect("link local Library resource");
    assert_eq!(outcome.completion, "open");
    let registration = outcome.resource.registration;
    assert_eq!(registration.resource_id, local_resource_id);
    let link_id = outcome.link_id;
    let repository_id = outcome
        .resource
        .links
        .iter()
        .find(|link| link.link.link_id == link_id)
        .expect("link outcome contains created link")
        .link
        .repository_id
        .clone();
    LinkedResource {
        link_id,
        registration_id: registration.registration_id,
        repository_id,
        resource_id: registration.resource_id,
    }
}

fn adopt_remote_resource(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    repository_id: &CatalogResourceRepositoryId,
) -> LinkedResource {
    let page = replica
        .service
        .list_scope_resources(scope_id, Some(storage_id))
        .expect("list storage-only Library resource");
    let row = page
        .rows
        .iter()
        .find(|row| {
            row.repository_id.as_ref() == Some(repository_id) && row.remote_present && !row.linked
        })
        .unwrap_or_else(|| {
            panic!(
                "remote repository '{}' is absent from its Library scope",
                repository_id
            )
        });
    assert!(!row.local_present);
    let outcome = replica
        .service
        .link_scope_resource(LinkCatalogScopeResourceRequest {
            scope_id: scope_id.clone(),
            storage_id: storage_id.clone(),
            local_resource_id: None,
            repository_id: Some(repository_id.clone()),
        })
        .expect("adopt storage-only Library resource");
    let registration = outcome.resource.registration;
    LinkedResource {
        link_id: outcome.link_id,
        registration_id: registration.registration_id,
        repository_id: repository_id.clone(),
        resource_id: registration.resource_id,
    }
}

fn assert_link_state(replica: &Replica, linked: &LinkedResource, expected: CatalogLinkState) {
    let inspection = replica
        .service
        .inspect_links(&linked.registration_id)
        .expect("inspect Library link")
        .into_iter()
        .find(|inspection| inspection.link_id == linked.link_id)
        .expect("created link is inspectable");
    assert_eq!(inspection.state, expected);
    assert_eq!(inspection.repository_id, linked.repository_id);
}

fn push_resource(
    replica: &Replica,
    linked: &LinkedResource,
    expected_relation: CatalogResourceRelation,
    expected_revision: u64,
) -> CatalogResourceId {
    let plan = replica
        .service
        .prepare_push(&linked.link_id)
        .expect("prepare reviewed Library Push");
    assert_eq!(plan.schema_version, CATALOG_SCHEMA);
    assert_eq!(plan.row.relation, expected_relation);
    assert!(plan.row.selectable);
    assert!(plan.row.selected_by_default);
    assert!(!plan.row.held_for_pull);
    assert_eq!(plan.repository_id, linked.repository_id);
    let resource_id = plan.row.resource_id.clone();
    let mut approval = CatalogPublishApproval::recommended(&plan);
    assert!(approval.include);
    assert!(approval.acknowledged_warning_digests.is_empty());
    if matches!(plan.resource, CatalogResource::AgentSkill(_)) {
        let executable = plan
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
    if !plan.row.warnings.is_empty() {
        let error = replica
            .service
            .publish_reviewed(&linked.link_id, approval.clone())
            .expect_err("unacknowledged resource warning must block publication");
        assert!(
            error.contains("unacknowledged warning"),
            "unexpected warning acknowledgement error: {error}"
        );
        approval.acknowledged_warning_digests = plan
            .row
            .warnings
            .iter()
            .map(|warning| warning.digest.clone())
            .collect();
    }
    let outcome = replica
        .service
        .publish_reviewed(&linked.link_id, approval)
        .expect("publish reviewed Library resource");
    assert!(outcome.published);
    assert_eq!(outcome.revision, Some(expected_revision));
    resource_id
}

fn assert_pull_kind(kind: &CatalogPullActionKind, expected: PullMutation) {
    assert!(
        matches!(
            (kind, expected),
            (
                CatalogPullActionKind::InstallSkill,
                PullMutation::InstallSkill
            ) | (
                CatalogPullActionKind::UpdateSkill,
                PullMutation::UpdateSkill
            ) | (
                CatalogPullActionKind::PreserveSkillAsCopyAndOverwrite { .. },
                PullMutation::PreserveSkill
            ) | (CatalogPullActionKind::InstallPet, PullMutation::InstallPet)
                | (CatalogPullActionKind::UpdatePet, PullMutation::UpdatePet)
                | (
                    CatalogPullActionKind::PreservePetAsCopyAndOverwrite { .. },
                    PullMutation::PreservePet
                )
        ),
        "unexpected Pull action: {kind:?}"
    );
}

fn pull_resource(
    replica: &Replica,
    linked: &LinkedResource,
    expected_relation: CatalogResourceRelation,
    expected_mutation: PullMutation,
    copy_name: Option<&str>,
    expected_outcome: CatalogActionOutcome,
) {
    let plan = replica
        .service
        .prepare_pull(&linked.link_id, copy_name)
        .expect("prepare reviewed Library Pull");
    assert_eq!(plan.schema_version, CATALOG_SCHEMA);
    assert_eq!(plan.repository_id, linked.repository_id);
    assert_eq!(plan.item.relation, expected_relation);
    assert_eq!(plan.item.disposition, CatalogPullDisposition::Executable);
    let action_id = plan
        .item
        .default_action_id
        .as_ref()
        .expect("mutating Pull has a recommended action");
    let action = plan
        .item
        .actions
        .iter()
        .find(|action| &action.action_id == action_id)
        .expect("recommended Pull action exists");
    assert_pull_kind(&action.kind, expected_mutation);
    if let Some(copy_name) = copy_name {
        let preservation = action
            .kind
            .preservation()
            .expect("divergent Pull defaults to preservation");
        assert_eq!(preservation.copy_name, copy_name);
        assert!(!action.requires_destructive_approval);
        let destructive = plan
            .item
            .actions
            .iter()
            .find(|candidate| candidate.kind.is_destructive_overwrite())
            .expect("divergent Pull review lacks its explicit overwrite alternative");
        assert!(destructive.requires_destructive_approval);
        let error = replica
            .service
            .apply_pull(
                &linked.link_id,
                CatalogPullApproval {
                    plan_id: plan.plan_id.clone(),
                    approved_action_id: Some(destructive.action_id.clone()),
                    destructive_acknowledged: false,
                },
            )
            .expect_err("destructive overwrite without acknowledgement must fail");
        assert!(
            error.contains("requires explicit acknowledgement"),
            "unexpected destructive acknowledgement error: {error}"
        );
    }
    let outcome = replica
        .service
        .apply_pull(&linked.link_id, CatalogPullApproval::recommended(&plan))
        .expect("apply reviewed Library Pull");
    assert!(outcome.remote_accepted);
    assert_eq!(outcome.result.outcome, expected_outcome);
}

fn assert_stale_push_rejected(
    replica: &Replica,
    linked: &LinkedResource,
    stale_plan: tauri_app_lib::CatalogResourcePushPlan,
) {
    let approval = CatalogPublishApproval {
        plan_id: stale_plan.plan_id.clone(),
        include: stale_plan.row.selectable,
        acknowledged_warning_digests: stale_plan
            .row
            .warnings
            .iter()
            .map(|warning| warning.digest.clone())
            .collect(),
    };
    let error = replica
        .service
        .publish_reviewed(&linked.link_id, approval)
        .expect_err("stale reviewed Push must fail");
    assert!(
        error.contains("storage head changed after review"),
        "unexpected stale Push error: {error}"
    );
}

fn assert_divergent_push_held(replica: &Replica, linked: &LinkedResource) {
    let plan = replica
        .service
        .prepare_push(&linked.link_id)
        .expect("prepare divergent Library Push review");
    assert_eq!(plan.row.relation, CatalogResourceRelation::ChangedBoth);
    assert!(!plan.row.selectable);
    assert!(!plan.row.selected_by_default);
    assert!(plan.row.held_for_pull);
    let error = replica
        .service
        .publish_reviewed(
            &linked.link_id,
            CatalogPublishApproval {
                plan_id: plan.plan_id.clone(),
                include: true,
                acknowledged_warning_digests: plan
                    .row
                    .warnings
                    .iter()
                    .map(|warning| warning.digest.clone())
                    .collect(),
            },
        )
        .expect_err("divergent Push cannot bypass Pull review");
    assert!(error.contains("Pull") || error.contains("safe to Push"));
}

fn adopt_and_install(
    replica: &Replica,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    repository_id: &CatalogResourceRepositoryId,
    expected_mutation: PullMutation,
) -> LinkedResource {
    let linked = adopt_remote_resource(replica, scope_id, storage_id, repository_id);
    pull_resource(
        replica,
        &linked,
        CatalogResourceRelation::StorageOnly,
        expected_mutation,
        None,
        CatalogActionOutcome::Installed,
    );
    linked
}

fn run_skill_flow(
    a: &Replica,
    b: &Replica,
    storage_id: &StorageId,
) -> (Vec<SyncedResource>, Vec<u64>, Vec<u64>) {
    let alpha_v1 = write_skill(a, ALPHA, "alpha-v1-from-a");
    let beta_v1 = write_skill(b, BETA, "beta-v1-from-b");

    let alpha_a = link_local_resource(a, &a.scopes.skills, storage_id, &skill_path(a, ALPHA));
    let alpha_resource_id = push_resource(a, &alpha_a, CatalogResourceRelation::LocalOnly, 1);
    assert_eq!(alpha_resource_id, alpha_a.resource_id);
    let alpha_b = adopt_and_install(
        b,
        &b.scopes.skills,
        storage_id,
        &alpha_a.repository_id,
        PullMutation::InstallSkill,
    );
    assert_eq!(alpha_b.resource_id, alpha_resource_id);
    assert_tree(&skill_path(b, ALPHA), &alpha_v1);

    let beta_b = link_local_resource(b, &b.scopes.skills, storage_id, &skill_path(b, BETA));
    let beta_resource_id = push_resource(b, &beta_b, CatalogResourceRelation::LocalOnly, 1);
    assert_eq!(beta_resource_id, beta_b.resource_id);
    let beta_a = adopt_and_install(
        a,
        &a.scopes.skills,
        storage_id,
        &beta_b.repository_id,
        PullMutation::InstallSkill,
    );
    assert_eq!(beta_a.resource_id, beta_resource_id);
    assert_tree(&skill_path(a, BETA), &beta_v1);

    let alpha_v2 = write_skill(a, ALPHA, "alpha-v2-from-a");
    assert_link_state(a, &alpha_a, CatalogLinkState::LocalChanged);
    assert_eq!(
        push_resource(a, &alpha_a, CatalogResourceRelation::LocalChanged, 2),
        alpha_resource_id
    );
    assert_link_state(b, &alpha_b, CatalogLinkState::StorageChanged);
    pull_resource(
        b,
        &alpha_b,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdateSkill,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(b, ALPHA), &alpha_v2);

    let beta_v2 = write_skill(b, BETA, "beta-v2-from-b");
    assert_link_state(b, &beta_b, CatalogLinkState::LocalChanged);
    assert_eq!(
        push_resource(b, &beta_b, CatalogResourceRelation::LocalChanged, 2),
        beta_resource_id
    );
    assert_link_state(a, &beta_a, CatalogLinkState::StorageChanged);
    pull_resource(
        a,
        &beta_a,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdateSkill,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(a, BETA), &beta_v2);

    // Alpha diverges. A wins the storage race; B's local branch is preserved.
    let alpha_a_winner = write_skill(a, ALPHA, "alpha-v3-a-storage-winner");
    let alpha_b_loser = write_skill(b, ALPHA, "alpha-v3-b-preserved-local");
    let alpha_b_stale = b
        .service
        .prepare_push(&alpha_b.link_id)
        .expect("prepare alpha losing Push");
    assert_eq!(
        alpha_b_stale.row.relation,
        CatalogResourceRelation::LocalChanged
    );
    assert_eq!(
        push_resource(a, &alpha_a, CatalogResourceRelation::LocalChanged, 3),
        alpha_resource_id
    );
    assert_stale_push_rejected(b, &alpha_b, alpha_b_stale);
    assert_divergent_push_held(b, &alpha_b);
    pull_resource(
        b,
        &alpha_b,
        CatalogResourceRelation::ChangedBoth,
        PullMutation::PreserveSkill,
        Some(ALPHA_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&skill_path(b, ALPHA), &alpha_a_winner);
    let alpha_copy_snapshot = renamed_skill_snapshot(&alpha_b_loser, ALPHA_COPY);
    assert_tree(&skill_path(b, ALPHA_COPY), &alpha_copy_snapshot);

    let alpha_copy_b =
        link_local_resource(b, &b.scopes.skills, storage_id, &skill_path(b, ALPHA_COPY));
    let alpha_copy_resource_id =
        push_resource(b, &alpha_copy_b, CatalogResourceRelation::LocalOnly, 1);
    assert_ne!(alpha_copy_resource_id, alpha_resource_id);
    assert_ne!(alpha_copy_b.repository_id, alpha_b.repository_id);
    let alpha_copy_a = adopt_and_install(
        a,
        &a.scopes.skills,
        storage_id,
        &alpha_copy_b.repository_id,
        PullMutation::InstallSkill,
    );
    assert_eq!(alpha_copy_a.resource_id, alpha_copy_resource_id);
    assert_tree(&skill_path(a, ALPHA_COPY), &alpha_copy_snapshot);

    // Beta diverges in the opposite direction. B wins; A is preserved.
    let beta_a_loser = write_skill(a, BETA, "beta-v3-a-preserved-local");
    let beta_b_winner = write_skill(b, BETA, "beta-v3-b-storage-winner");
    let beta_a_stale = a
        .service
        .prepare_push(&beta_a.link_id)
        .expect("prepare beta losing Push");
    assert_eq!(
        beta_a_stale.row.relation,
        CatalogResourceRelation::LocalChanged
    );
    assert_eq!(
        push_resource(b, &beta_b, CatalogResourceRelation::LocalChanged, 3),
        beta_resource_id
    );
    assert_stale_push_rejected(a, &beta_a, beta_a_stale);
    assert_divergent_push_held(a, &beta_a);
    pull_resource(
        a,
        &beta_a,
        CatalogResourceRelation::ChangedBoth,
        PullMutation::PreserveSkill,
        Some(BETA_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&skill_path(a, BETA), &beta_b_winner);
    let beta_copy_snapshot = renamed_skill_snapshot(&beta_a_loser, BETA_COPY);
    assert_tree(&skill_path(a, BETA_COPY), &beta_copy_snapshot);

    let beta_copy_a =
        link_local_resource(a, &a.scopes.skills, storage_id, &skill_path(a, BETA_COPY));
    let beta_copy_resource_id =
        push_resource(a, &beta_copy_a, CatalogResourceRelation::LocalOnly, 1);
    assert_ne!(beta_copy_resource_id, beta_resource_id);
    assert_ne!(beta_copy_a.repository_id, beta_a.repository_id);
    let beta_copy_b = adopt_and_install(
        b,
        &b.scopes.skills,
        storage_id,
        &beta_copy_a.repository_id,
        PullMutation::InstallSkill,
    );
    assert_eq!(beta_copy_b.resource_id, beta_copy_resource_id);
    assert_tree(&skill_path(b, BETA_COPY), &beta_copy_snapshot);

    // A preserved conflict does not poison the primary link: the former
    // losing replica can publish a normal next revision in either direction.
    let alpha_v4 = write_skill(b, ALPHA, "alpha-v4-from-former-loser-b");
    assert_eq!(
        push_resource(b, &alpha_b, CatalogResourceRelation::LocalChanged, 4),
        alpha_resource_id
    );
    pull_resource(
        a,
        &alpha_a,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdateSkill,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(a, ALPHA), &alpha_v4);

    let beta_v4 = write_skill(a, BETA, "beta-v4-from-former-loser-a");
    assert_eq!(
        push_resource(a, &beta_a, CatalogResourceRelation::LocalChanged, 4),
        beta_resource_id
    );
    pull_resource(
        b,
        &beta_b,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdateSkill,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&skill_path(b, BETA), &beta_v4);

    (
        vec![
            SyncedResource {
                a_link: alpha_a.link_id,
                b_link: alpha_b.link_id,
                repository_id: alpha_a.repository_id,
                resource_id: alpha_resource_id,
            },
            SyncedResource {
                a_link: beta_a.link_id,
                b_link: beta_b.link_id,
                repository_id: beta_a.repository_id,
                resource_id: beta_resource_id,
            },
            SyncedResource {
                a_link: alpha_copy_a.link_id,
                b_link: alpha_copy_b.link_id,
                repository_id: alpha_copy_a.repository_id,
                resource_id: alpha_copy_resource_id,
            },
            SyncedResource {
                a_link: beta_copy_a.link_id,
                b_link: beta_copy_b.link_id,
                repository_id: beta_copy_a.repository_id,
                resource_id: beta_copy_resource_id,
            },
        ],
        vec![4, 4],
        vec![1, 1],
    )
}

fn run_pet_flow(
    a: &Replica,
    b: &Replica,
    storage_id: &StorageId,
) -> (Vec<SyncedResource>, Vec<u64>, Vec<u64>) {
    let duck_v1 = write_pet(
        a,
        CodexPetFormat::V1,
        DUCK,
        "Mallard Duck",
        "duck-v1-from-a",
        "duck.webp",
        1,
    );
    let owl_v1 = write_pet(
        b,
        CodexPetFormat::V2,
        OWL,
        "Mallard Owl",
        "owl-v1-from-b",
        "owl.webp",
        11,
    );

    let duck_a = link_local_resource(a, &a.scopes.pets_v1, storage_id, &pet_path(a, DUCK));
    let duck_resource_id = push_resource(a, &duck_a, CatalogResourceRelation::LocalOnly, 1);
    let duck_b = adopt_and_install(
        b,
        &b.scopes.pets_v1,
        storage_id,
        &duck_a.repository_id,
        PullMutation::InstallPet,
    );
    assert_eq!(duck_a.resource_id, duck_resource_id);
    assert_eq!(duck_b.resource_id, duck_resource_id);
    assert_tree(&pet_path(b, DUCK), &duck_v1);

    let owl_b = link_local_resource(b, &b.scopes.pets_v2, storage_id, &pet_path(b, OWL));
    let owl_resource_id = push_resource(b, &owl_b, CatalogResourceRelation::LocalOnly, 1);
    let owl_a = adopt_and_install(
        a,
        &a.scopes.pets_v2,
        storage_id,
        &owl_b.repository_id,
        PullMutation::InstallPet,
    );
    assert_eq!(owl_a.resource_id, owl_resource_id);
    assert_eq!(owl_b.resource_id, owl_resource_id);
    assert_ne!(duck_resource_id, owl_resource_id);
    assert_tree(&pet_path(a, OWL), &owl_v1);

    // The receiving replica may publish the next ordinary revision.
    let duck_v2 = write_pet(
        b,
        CodexPetFormat::V1,
        DUCK,
        "Mallard Duck",
        "duck-v2-from-receiver-b",
        "duck.webp",
        2,
    );
    assert_eq!(
        push_resource(b, &duck_b, CatalogResourceRelation::LocalChanged, 2),
        duck_resource_id
    );
    pull_resource(
        a,
        &duck_a,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdatePet,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(a, DUCK), &duck_v2);

    let owl_v2 = write_pet(
        a,
        CodexPetFormat::V2,
        OWL,
        "Mallard Owl",
        "owl-v2-from-receiver-a",
        "owl.webp",
        12,
    );
    assert_eq!(
        push_resource(a, &owl_a, CatalogResourceRelation::LocalChanged, 2),
        owl_resource_id
    );
    pull_resource(
        b,
        &owl_b,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdatePet,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(b, OWL), &owl_v2);

    // Duck divergence: A wins the remote head and B preserves its local pet.
    let duck_a_winner = write_pet(
        a,
        CodexPetFormat::V1,
        DUCK,
        "Mallard Duck",
        "duck-v3-a-storage-winner",
        "duck.webp",
        3,
    );
    let duck_b_loser = write_pet(
        b,
        CodexPetFormat::V1,
        DUCK,
        "Mallard Duck",
        "duck-v3-b-preserved-local",
        "duck.webp",
        4,
    );
    let duck_b_stale = b
        .service
        .prepare_push(&duck_b.link_id)
        .expect("prepare duck losing Push");
    assert_eq!(
        push_resource(a, &duck_a, CatalogResourceRelation::LocalChanged, 3),
        duck_resource_id
    );
    assert_stale_push_rejected(b, &duck_b, duck_b_stale);
    assert_divergent_push_held(b, &duck_b);
    pull_resource(
        b,
        &duck_b,
        CatalogResourceRelation::ChangedBoth,
        PullMutation::PreservePet,
        Some(DUCK_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&pet_path(b, DUCK), &duck_a_winner);
    let duck_copy_snapshot = renamed_pet_snapshot(
        &duck_b_loser,
        CodexPetFormat::V1,
        DUCK_COPY,
        "duck-v3-b-preserved-local",
        "duck.webp",
    );
    assert_tree(&pet_path(b, DUCK_COPY), &duck_copy_snapshot);

    let duck_copy_b =
        link_local_resource(b, &b.scopes.pets_v1, storage_id, &pet_path(b, DUCK_COPY));
    let duck_copy_resource_id =
        push_resource(b, &duck_copy_b, CatalogResourceRelation::LocalOnly, 1);
    assert_ne!(duck_copy_resource_id, duck_resource_id);
    assert_ne!(duck_copy_b.repository_id, duck_b.repository_id);
    let duck_copy_a = adopt_and_install(
        a,
        &a.scopes.pets_v1,
        storage_id,
        &duck_copy_b.repository_id,
        PullMutation::InstallPet,
    );
    assert_eq!(duck_copy_a.resource_id, duck_copy_resource_id);
    assert_tree(&pet_path(a, DUCK_COPY), &duck_copy_snapshot);

    // Owl divergence reverses the winner/loser roles and preserves v2.
    let owl_a_loser = write_pet(
        a,
        CodexPetFormat::V2,
        OWL,
        "Mallard Owl",
        "owl-v3-a-preserved-local",
        "owl.webp",
        13,
    );
    let owl_b_winner = write_pet(
        b,
        CodexPetFormat::V2,
        OWL,
        "Mallard Owl",
        "owl-v3-b-storage-winner",
        "owl.webp",
        14,
    );
    let owl_a_stale = a
        .service
        .prepare_push(&owl_a.link_id)
        .expect("prepare owl losing Push");
    assert_eq!(
        push_resource(b, &owl_b, CatalogResourceRelation::LocalChanged, 3),
        owl_resource_id
    );
    assert_stale_push_rejected(a, &owl_a, owl_a_stale);
    assert_divergent_push_held(a, &owl_a);
    pull_resource(
        a,
        &owl_a,
        CatalogResourceRelation::ChangedBoth,
        PullMutation::PreservePet,
        Some(OWL_COPY),
        CatalogActionOutcome::PreservedAndUpdated,
    );
    assert_tree(&pet_path(a, OWL), &owl_b_winner);
    let owl_copy_snapshot = renamed_pet_snapshot(
        &owl_a_loser,
        CodexPetFormat::V2,
        OWL_COPY,
        "owl-v3-a-preserved-local",
        "owl.webp",
    );
    assert_tree(&pet_path(a, OWL_COPY), &owl_copy_snapshot);

    let owl_copy_a = link_local_resource(a, &a.scopes.pets_v2, storage_id, &pet_path(a, OWL_COPY));
    let owl_copy_resource_id = push_resource(a, &owl_copy_a, CatalogResourceRelation::LocalOnly, 1);
    assert_ne!(owl_copy_resource_id, owl_resource_id);
    assert_ne!(owl_copy_a.repository_id, owl_a.repository_id);
    let owl_copy_b = adopt_and_install(
        b,
        &b.scopes.pets_v2,
        storage_id,
        &owl_copy_a.repository_id,
        PullMutation::InstallPet,
    );
    assert_eq!(owl_copy_b.resource_id, owl_copy_resource_id);
    assert_tree(&pet_path(b, OWL_COPY), &owl_copy_snapshot);

    let duck_v4 = write_pet(
        b,
        CodexPetFormat::V1,
        DUCK,
        "Mallard Duck",
        "duck-v4-from-former-loser-b",
        "duck.webp",
        5,
    );
    assert_eq!(
        push_resource(b, &duck_b, CatalogResourceRelation::LocalChanged, 4),
        duck_resource_id
    );
    pull_resource(
        a,
        &duck_a,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdatePet,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(a, DUCK), &duck_v4);

    let owl_v4 = write_pet(
        a,
        CodexPetFormat::V2,
        OWL,
        "Mallard Owl",
        "owl-v4-from-former-loser-a",
        "owl.webp",
        15,
    );
    assert_eq!(
        push_resource(a, &owl_a, CatalogResourceRelation::LocalChanged, 4),
        owl_resource_id
    );
    pull_resource(
        b,
        &owl_b,
        CatalogResourceRelation::StorageChanged,
        PullMutation::UpdatePet,
        None,
        CatalogActionOutcome::Updated,
    );
    assert_tree(&pet_path(b, OWL), &owl_v4);

    (
        vec![
            SyncedResource {
                a_link: duck_a.link_id,
                b_link: duck_b.link_id,
                repository_id: duck_a.repository_id,
                resource_id: duck_resource_id,
            },
            SyncedResource {
                a_link: owl_a.link_id,
                b_link: owl_b.link_id,
                repository_id: owl_a.repository_id,
                resource_id: owl_resource_id,
            },
            SyncedResource {
                a_link: duck_copy_a.link_id,
                b_link: duck_copy_b.link_id,
                repository_id: duck_copy_a.repository_id,
                resource_id: duck_copy_resource_id,
            },
            SyncedResource {
                a_link: owl_copy_a.link_id,
                b_link: owl_copy_b.link_id,
                repository_id: owl_copy_a.repository_id,
                resource_id: owl_copy_resource_id,
            },
        ],
        vec![4, 4],
        vec![1, 1],
    )
}

fn assert_repository(
    replica: &Replica,
    storage_id: &StorageId,
    resource: &SyncedResource,
    expected_revision: u64,
) {
    let inspection = replica
        .service
        .inspect_repository(storage_id, &resource.repository_id)
        .expect("inspect immutable Library repository");
    let tag = inspection.tag.expect("published repository has a tag");
    let head = inspection.head.expect("published repository has a head");
    let stored = inspection
        .resource
        .expect("published repository has a resource manifest");
    assert_eq!(tag.schema_version, CATALOG_SCHEMA);
    assert_eq!(tag.repository_id, resource.repository_id);
    assert_eq!(tag.resource_id, resource.resource_id);
    assert_eq!(head.repository_id, resource.repository_id);
    assert_eq!(head.revision, expected_revision);
    assert_eq!(stored.resource_id(), resource.resource_id);
}

fn assert_noop_fixed_point(
    replica: &Replica,
    link_id: &CatalogLinkId,
    resource_id: &CatalogResourceId,
    expected_revision: u64,
) {
    let push = replica
        .service
        .prepare_push(link_id)
        .expect("prepare fixed-point Push");
    assert_eq!(push.row.relation, CatalogResourceRelation::Matching);
    assert_eq!(&push.row.resource_id, resource_id);
    assert!(!push.row.selectable);
    assert!(!push.row.selected_by_default);
    assert!(!push.row.held_for_pull);
    let outcome = replica
        .service
        .publish_reviewed(link_id, CatalogPublishApproval::recommended(&push))
        .expect("skip fixed-point Push");
    assert!(!outcome.published);
    assert_eq!(outcome.revision, Some(expected_revision));

    let pull = replica
        .service
        .prepare_pull(link_id, None)
        .expect("prepare fixed-point Pull");
    assert_eq!(pull.item.relation, CatalogResourceRelation::Matching);
    assert_eq!(
        pull.item.disposition,
        CatalogPullDisposition::AlreadyMatching
    );
    assert!(pull.item.actions.is_empty());
    assert!(pull.item.default_action_id.is_none());
    let outcome = replica
        .service
        .apply_pull(link_id, CatalogPullApproval::recommended(&pull))
        .expect("apply fixed-point Pull");
    assert!(outcome.remote_accepted);
    assert_eq!(
        outcome.result.outcome,
        CatalogActionOutcome::AlreadyMatching
    );

    let after = replica
        .service
        .prepare_push(link_id)
        .expect("reinspect fixed-point link after no-op Pull");
    assert_eq!(after.row.relation, CatalogResourceRelation::Matching);
    assert_eq!(after.link_revision, push.link_revision);
    assert!(!after.row.selectable);
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
    assert_eq!(repositories.len(), 8);

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
pub fn run_catalog_flow(inputs: CatalogFlowInputs<'_>) -> CatalogFlowReport {
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
    let storage_root = Path::new(&inputs.storage.local_dir);
    assert!(storage_root.is_absolute());
    fs::create_dir_all(storage_root).expect("create shared local-folder storage");

    let calls = Arc::new(AtomicUsize::new(0));
    let a = create_replica(
        inputs.machine_home_a,
        inputs.user_home_a,
        inputs.codex_home_a,
        inputs.storage,
        calls.clone(),
        "myconf1",
    );
    let b = create_replica(
        inputs.machine_home_b,
        inputs.user_home_b,
        inputs.codex_home_b,
        inputs.storage,
        calls.clone(),
        "myconf2",
    );

    let (mut resources, mut primary_revisions, mut copy_revisions) =
        run_skill_flow(&a, &b, &inputs.storage.id);
    let injected_inventory_calls = calls.load(Ordering::Relaxed);
    assert!(
        injected_inventory_calls > 0,
        "skill ownership was not checked through the injected runner"
    );
    let (pet_resources, pet_primary_revisions, pet_copy_revisions) =
        run_pet_flow(&a, &b, &inputs.storage.id);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        injected_inventory_calls,
        "pet inventory unexpectedly launched the provider ownership runner"
    );
    resources.extend(pet_resources);
    primary_revisions.extend(pet_primary_revisions);
    copy_revisions.extend(pet_copy_revisions);
    assert_eq!(resources.len(), 8);
    assert_eq!(
        resources
            .iter()
            .map(|resource| &resource.repository_id)
            .collect::<BTreeSet<_>>()
            .len(),
        8,
        "each primary and preserved copy owns an independent repository"
    );
    assert_eq!(
        resources
            .iter()
            .map(|resource| &resource.resource_id)
            .collect::<BTreeSet<_>>()
            .len(),
        8,
        "each primary and preserved copy owns an independent resource identity"
    );

    assert_final_local_trees(&a, &b);
    let local_before_noops = final_local_tree_inventory(&a, &b);
    let before_noops = storage_snapshot(storage_root);
    for (index, resource) in resources.iter().enumerate() {
        let expected_revision = if index < 2 || (4..6).contains(&index) {
            4
        } else {
            1
        };
        assert_repository(&a, &inputs.storage.id, resource, expected_revision);
        assert_repository(&b, &inputs.storage.id, resource, expected_revision);
        assert_noop_fixed_point(
            &a,
            &resource.a_link,
            &resource.resource_id,
            expected_revision,
        );
        assert_noop_fixed_point(
            &b,
            &resource.b_link,
            &resource.resource_id,
            expected_revision,
        );
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

    primary_revisions.sort_unstable();
    copy_revisions.sort_unstable();
    CatalogFlowReport {
        repository_count,
        primary_revisions,
        preserved_copy_revisions: copy_revisions,
        // The injected runner returns bounded fixture JSON; no native Codex
        // process and no model request is launched by this flow.
        native_provider_process_calls: 0,
        injected_inventory_calls,
    }
}
