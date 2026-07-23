//! Schema-4/schema-5 bundle storage, publication, fetch, restore planning, and apply.
//!
//! The engine operates on validated logical/store keys and a small object
//! store trait. Its local implementation is complete; an S3 adapter can reuse
//! the same immutable-object and head-CAS protocol without exposing UI paths
//! to manifests.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use super::conversation_codec::{
    analyze_codex_transcript, CodecLineage, CodecUnknownReason, CODEX_CODEC_VERSION,
    CODEX_LINEAGE_CODEC_KEY, CODEX_REWRITE_CODEC_KEY, CODEX_SEMANTIC_CODEC_KEY,
};
use super::domain::{
    self, ActionId, ActionStatus, ApplyPolicy, ApplyReceipt, BindingState, BundleDirectoryEntry,
    BundleFileEntry, BundleHead, BundleId, BundleIdentity, BundleKind, BundleManifest,
    BundleRecipe, BundleSnapshot, CapturedWith, ConflictGroupId, ConversationFileRef,
    ConversationFileRole, ConversationGroup, ConversationLineageStatus, ConversationVariant,
    DependencyAction, DependencyActionKind, LogicalPath, PlanId, PortableOrigin, ProjectBinding,
    Provider, RecipeEntry, ResourceDescriptor, ResourceId, ResourceKind, RestoreAction,
    RestoreActionKind, RestorePlan, StorageId, Tombstone, TombstoneTarget, BUNDLE_SCHEMA_V4,
    BUNDLE_SCHEMA_V5, RESTORE_PLAN_SCHEMA_V2,
};
use super::provider_capture::{self, CapturedResources};

const MALLARD_ROOT: &str = ".mallard";
const STORAGE_MARKER_KEY: &str = ".mallard/_storage.json";
const REPOSITORY_PREFIX: &str = ".mallard/v1/repositories";
const STORAGE_MARKER_FORMAT: &str = "mallard-storage";
const STORAGE_LAYOUT_VERSION: u32 = 1;
const LOCAL_STORAGE_LOCK: &str = ".storage.lock";
const HEAD_FILE: &str = "_head.json";
const TAG_FILE: &str = "_tag.json";
const MAX_OBJECT_BYTES: usize = 512 * 1024 * 1024;
const MAX_LIST_PAGE: usize = 10_000;
const DEFAULT_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_store_path(&value, 2)?;
        if value == STORAGE_MARKER_KEY {
            return Ok(Self(value));
        }
        validate_store_path(&value, 5)?;
        if !value.starts_with(&format!("{}/", REPOSITORY_PREFIX)) {
            return Err(format!(
                "object key is outside the Mallard storage namespace: '{}'",
                value
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPrefix(String);

impl ObjectPrefix {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into().trim_end_matches('/').to_string();
        validate_store_path(&value, 3)?;
        if value != REPOSITORY_PREFIX && !value.starts_with(&format!("{}/", REPOSITORY_PREFIX)) {
            return Err(format!(
                "object prefix is outside {}: '{}'",
                REPOSITORY_PREFIX, value
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    pub bytes: Vec<u8>,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasExpectation {
    Absent,
    Match(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasOutcome {
    Written { etag: String },
    Conflict { current_etag: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImmutablePutOutcome {
    Written,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreListPage {
    pub keys: Vec<ObjectKey>,
    pub next_cursor: Option<String>,
}

/// The narrow transport contract required by the bundle protocol.
pub trait BundleObjectStore: Send + Sync {
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String>;
    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String>;
    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String>;
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String>;

    /// Used only for local isolation checks; remote stores return `None`.
    fn local_root(&self) -> Option<&Path> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct LocalBundleObjectStore {
    root: PathBuf,
}

impl LocalBundleObjectStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(format!(
                "local bundle store must be absolute: '{}'",
                root.display()
            ));
        }
        if let Ok(meta) = fs::symlink_metadata(root) {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!(
                    "local store '{}' must be a real directory",
                    root.display()
                ));
            }
        } else {
            fs::create_dir_all(root)
                .map_err(|e| format!("create local store '{}': {}", root.display(), e))?;
        }
        let root = fs::canonicalize(root)
            .map_err(|e| format!("resolve local store '{}': {}", root.display(), e))?;
        let mallard_root = root.join(MALLARD_ROOT);
        match fs::symlink_metadata(&mallard_root) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "Mallard storage namespace '{}' must be a real directory",
                    mallard_root.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&mallard_root) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!(
                            "create Mallard storage namespace '{}': {}",
                            mallard_root.display(),
                            error
                        ))
                    }
                }
                let meta = fs::symlink_metadata(&mallard_root).map_err(|error| {
                    format!(
                        "inspect Mallard storage namespace '{}': {}",
                        mallard_root.display(),
                        error
                    )
                })?;
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err(format!(
                        "Mallard storage namespace '{}' must be a real directory",
                        mallard_root.display()
                    ));
                }
            }
            Err(error) => {
                return Err(format!(
                    "inspect Mallard storage namespace '{}': {}",
                    mallard_root.display(),
                    error
                ))
            }
        }
        Ok(Self { root })
    }

    fn lock(&self) -> Result<fs::File, String> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(MALLARD_ROOT).join(LOCAL_STORAGE_LOCK))
            .map_err(|e| format!("open Mallard storage lock: {}", e))?;
        file.lock()
            .map_err(|e| format!("lock local Mallard storage: {}", e))?;
        Ok(file)
    }

    fn read_unlocked(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        let path = checked_existing_object_path(&self.root, key)?;
        let Some(path) = path else {
            return Ok(None);
        };
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("inspect '{}': {}", key, e))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("object '{}' is not a regular no-follow file", key));
        }
        if meta.len() > MAX_OBJECT_BYTES as u64 {
            return Err(format!("object '{}' exceeds the read limit", key));
        }
        let bytes = fs::read(&path).map_err(|e| format!("read '{}': {}", key, e))?;
        Ok(Some(StoredObject {
            etag: sha256(&bytes),
            bytes,
        }))
    }

    fn write_atomic_unlocked(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(format!("object '{}' exceeds the write limit", key));
        }
        let path = checked_create_object_path(&self.root, key)?;
        let parent = path
            .parent()
            .ok_or_else(|| format!("object '{}' has no parent", key))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("create object temp in '{}': {}", parent.display(), e))?;
        temp.as_file_mut()
            .write_all(bytes)
            .map_err(|e| format!("write object '{}': {}", key, e))?;
        temp.as_file_mut()
            .sync_all()
            .map_err(|e| format!("sync object '{}': {}", key, e))?;
        temp.persist(&path)
            .map_err(|e| format!("publish object '{}': {}", key, e.error))?;
        Ok(())
    }
}

impl BundleObjectStore for LocalBundleObjectStore {
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        self.read_unlocked(key)
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        let _guard = self.lock()?;
        if let Some(existing) = self.read_unlocked(key)? {
            if existing.bytes == bytes {
                return Ok(ImmutablePutOutcome::AlreadyPresent);
            }
            return Err(format!(
                "immutable object '{}' already exists with different bytes",
                key
            ));
        }
        self.write_atomic_unlocked(key, bytes)?;
        Ok(ImmutablePutOutcome::Written)
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        let _guard = self.lock()?;
        let current = self.read_unlocked(key)?;
        let matches = match expectation {
            CasExpectation::Absent => current.is_none(),
            CasExpectation::Match(expected) => current
                .as_ref()
                .is_some_and(|object| object.etag == *expected),
        };
        if !matches {
            return Ok(CasOutcome::Conflict {
                current_etag: current.map(|object| object.etag),
            });
        }
        self.write_atomic_unlocked(key, bytes)?;
        Ok(CasOutcome::Written {
            etag: sha256(bytes),
        })
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(format!(
                "list limit must be between 1 and {}",
                MAX_LIST_PAGE
            ));
        }
        if let Some(cursor) = cursor {
            ObjectKey::parse(cursor.to_string())?;
        }
        let namespace_root = self.root.join(prefix.as_str());
        let namespace_meta = match fs::symlink_metadata(&namespace_root) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreListPage {
                    keys: Vec::new(),
                    next_cursor: None,
                })
            }
            Err(error) => {
                return Err(format!(
                    "inspect local Mallard repository namespace '{}': {}",
                    namespace_root.display(),
                    error
                ))
            }
        };
        if namespace_meta.file_type().is_symlink() || !namespace_meta.is_dir() {
            return Err(format!(
                "local Mallard repository namespace '{}' must be a real directory",
                namespace_root.display()
            ));
        }

        let mut keys = Vec::new();
        for entry in WalkDir::new(&namespace_root)
            .follow_links(false)
            .into_iter()
        {
            let entry = entry.map_err(|e| format!("walk local bundle store: {}", e))?;
            if entry.depth() == 0 {
                continue;
            }
            if entry.file_type().is_symlink() {
                return Err(format!(
                    "symlink '{}' is forbidden in local bundle storage",
                    entry.path().display()
                ));
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = normalized_relative_path(&self.root, entry.path())?;
            if relative.contains("/.tmp") {
                continue;
            }
            if relative == prefix.as_str() || relative.starts_with(&format!("{}/", prefix.as_str()))
            {
                let key = ObjectKey::parse(relative)?;
                if cursor.is_none_or(|cursor| key.as_str() > cursor) {
                    keys.push(key);
                }
            }
        }
        keys.sort();
        let has_more = keys.len() > limit;
        keys.truncate(limit);
        let next_cursor = has_more.then(|| keys.last().unwrap().as_str().to_string());
        Ok(StoreListPage { keys, next_cursor })
    }

    fn local_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadToken {
    pub generation: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishExpectation {
    Absent,
    Match(HeadToken),
}

#[derive(Clone, Debug)]
pub struct PublishBundleRequest {
    pub identity: BundleIdentity,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub captured: CapturedResources,
    pub expected_head: PublishExpectation,
    pub updated_at: u64,
}

#[derive(Clone, Debug)]
pub struct PublishedBundle {
    pub snapshot: BundleSnapshot,
    // Read by CAS-chaining tests; command flows re-read the head instead.
    #[cfg_attr(not(test), allow(dead_code))]
    pub head_token: HeadToken,
}

#[derive(Clone, Debug)]
pub struct Schema5CapturedFile {
    pub resource_id: ResourceId,
    pub bytes: Vec<u8>,
    pub source_mtime: u64,
    pub mode: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct PublishSchema5Request {
    pub identity: BundleIdentity,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub published_by_origin: PortableOrigin,
    pub conversation_groups: BTreeMap<ConflictGroupId, ConversationGroup>,
    pub resources: BTreeMap<ResourceId, ResourceDescriptor>,
    /// Entries copied byte-for-byte from the reviewed previous manifest. The
    /// engine verifies every referenced immutable object before publishing.
    pub retained_files: BTreeMap<LogicalPath, BundleFileEntry>,
    /// New or changed payloads. A path cannot also appear in retained_files.
    pub captured_files: BTreeMap<LogicalPath, Schema5CapturedFile>,
    pub directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    pub tombstones: BTreeMap<String, Tombstone>,
    pub expected_head: PublishExpectation,
    pub updated_at: u64,
}

#[derive(Clone, Debug)]
pub struct Schema5PublishOutcome {
    pub published: bool,
    pub snapshot: BundleSnapshot,
    pub head_token: HeadToken,
}

#[derive(Clone, Debug)]
pub struct FetchedBundle {
    pub snapshot: BundleSnapshot,
    pub files: BTreeMap<LogicalPath, Vec<u8>>,
    pub dependency_actions: Vec<DependencyAction>,
}

const CLAUDE_UNAVAILABLE_LINEAGE_CODEC_KEY: &str = "claude.lineage.unavailable.v1";

/// Build the one-way schema-4-to-schema-5 publication reviewed by a cutover
/// caller. Historical provider bytes are copied exactly, while conversation
/// identities and logical ownership move to the group/variant model. The
/// migrating host is deliberately not recorded as creator or semantic editor.
pub fn build_schema5_cutover_request(
    fetched: &FetchedBundle,
    published_by_origin: PortableOrigin,
    expected_head: HeadToken,
    updated_at: u64,
) -> Result<PublishSchema5Request, String> {
    fetched.snapshot.validate()?;
    published_by_origin.validate()?;
    if fetched.snapshot.manifest.schema_version != BUNDLE_SCHEMA_V4 {
        return Err("schema cutover requires a reviewed schema-4 manifest".to_string());
    }
    let reviewed_head = &fetched.snapshot.head;
    if expected_head.generation != reviewed_head.generation
        || expected_head.commit_id != reviewed_head.commit_id
        || expected_head.manifest_sha256 != reviewed_head.manifest_sha256
    {
        return Err("schema cutover head pin does not match the reviewed snapshot".to_string());
    }
    if updated_at == 0 {
        return Err("schema cutover timestamp must be non-zero".to_string());
    }

    let manifest = &fetched.snapshot.manifest;
    let mut recipe = manifest.recipe.clone();
    let mut captured_with = manifest.captured_with.clone();
    captured_with
        .codec_versions
        .insert(CODEX_LINEAGE_CODEC_KEY.to_string(), CODEX_CODEC_VERSION);
    captured_with
        .codec_versions
        .insert(CODEX_REWRITE_CODEC_KEY.to_string(), CODEX_CODEC_VERSION);
    captured_with
        .codec_versions
        .insert(CODEX_SEMANTIC_CODEC_KEY.to_string(), CODEX_CODEC_VERSION);

    let mut resources = manifest.resources.clone();
    let mut retained_files = BTreeMap::new();
    let mut captured_files = BTreeMap::new();
    let mut conversation_groups = BTreeMap::new();
    let conversation_resource_ids = manifest
        .resources
        .iter()
        .filter_map(|(resource_id, descriptor)| {
            matches!(
                descriptor.kind,
                ResourceKind::CodexConversation | ResourceKind::ClaudeConversation
            )
            .then_some(resource_id.clone())
        })
        .collect::<BTreeSet<_>>();

    for (path, entry) in &manifest.files {
        let bytes = fetched
            .files
            .get(path)
            .ok_or_else(|| format!("reviewed schema-4 file '{}' was not fetched", path))?;
        if bytes.len() as u64 != entry.size || sha256(bytes) != entry.sha256 {
            return Err(format!(
                "reviewed schema-4 file '{}' does not match its manifest digest",
                path
            ));
        }
        if !conversation_resource_ids.contains(&entry.resource_id) {
            retained_files.insert(path.clone(), entry.clone());
        }
    }
    if manifest
        .directories
        .values()
        .any(|entry| conversation_resource_ids.contains(&entry.resource_id))
    {
        return Err("schema-4 conversation resource unexpectedly owns a directory".to_string());
    }

    for legacy_resource_id in conversation_resource_ids {
        let legacy_descriptor = manifest
            .resources
            .get(&legacy_resource_id)
            .expect("conversation resource came from the manifest");
        let provider = legacy_descriptor.provider.ok_or_else(|| {
            format!(
                "legacy conversation '{}' has no provider",
                legacy_resource_id
            )
        })?;
        let (group_id, variant_id) = domain::deterministic_schema4_conversation_ids(
            &manifest.bundle.bundle_id,
            provider,
            &legacy_resource_id,
        )?;
        let resource_id = domain::conversation_resource_id(provider, &group_id, &variant_id)?;
        let mut owned = manifest
            .files
            .iter()
            .filter(|(_, entry)| entry.resource_id == legacy_resource_id)
            .collect::<Vec<_>>();
        owned.sort_by(|left, right| left.0.cmp(right.0));
        if owned.is_empty() {
            return Err(format!(
                "legacy conversation '{}' owns no transcript",
                legacy_resource_id
            ));
        }
        let transcript_index = owned
            .iter()
            .position(|(path, _)| path.as_str().ends_with(".jsonl"))
            .unwrap_or(0);
        let transcript_bytes = fetched
            .files
            .get(owned[transcript_index].0)
            .expect("owned legacy file was verified above");
        let analysis =
            (provider == Provider::Codex).then(|| analyze_codex_transcript(transcript_bytes));
        let (
            lineage_status,
            lineage_codec,
            head_message_id,
            message_count,
            lineage_digest,
            semantic_digest,
            source_thread_id,
        ) = match analysis {
            Some(analysis) => match analysis.lineage {
                CodecLineage::Complete {
                    chain,
                    lineage_digest,
                    semantic_digest,
                } => (
                    ConversationLineageStatus::Complete,
                    CODEX_LINEAGE_CODEC_KEY.to_string(),
                    chain.last().map(|message| message.message_id.clone()),
                    Some(chain.len() as u64),
                    Some(lineage_digest),
                    Some(semantic_digest),
                    analysis.source_thread_id,
                ),
                CodecLineage::Unknown { reason } => (
                    ConversationLineageStatus::Unknown {
                        reason_code: reason.as_str().to_string(),
                    },
                    CODEX_LINEAGE_CODEC_KEY.to_string(),
                    None,
                    None,
                    None,
                    None,
                    analysis.source_thread_id,
                ),
            },
            None => {
                captured_with
                    .codec_versions
                    .insert(CLAUDE_UNAVAILABLE_LINEAGE_CODEC_KEY.to_string(), 1);
                (
                    ConversationLineageStatus::Unknown {
                        reason_code: CodecUnknownReason::UnsupportedCodec.as_str().to_string(),
                    },
                    CLAUDE_UNAVAILABLE_LINEAGE_CODEC_KEY.to_string(),
                    None,
                    None,
                    None,
                    None,
                    legacy_resource_id
                        .as_str()
                        .rsplit(':')
                        .next()
                        .map(str::to_string),
                )
            }
        };

        let mut file_refs = Vec::with_capacity(owned.len());
        for (index, (old_path, old_entry)) in owned.into_iter().enumerate() {
            let (filename, role) = if index == transcript_index {
                (
                    "transcript.jsonl".to_string(),
                    ConversationFileRole::Transcript,
                )
            } else {
                (
                    format!("sidecar-{index}.json"),
                    ConversationFileRole::Sidecar {
                        sidecar_kind: format!("legacy-{index}"),
                    },
                )
            };
            let path = LogicalPath::parse(format!(
                "state/conversations/{}/{}/{}/{}",
                provider.as_str(),
                group_id,
                variant_id,
                filename
            ))?;
            let bytes = fetched
                .files
                .get(old_path)
                .expect("owned legacy file was verified above")
                .clone();
            captured_files.insert(
                path.clone(),
                Schema5CapturedFile {
                    resource_id: resource_id.clone(),
                    bytes,
                    source_mtime: old_entry.source_mtime,
                    mode: old_entry.mode,
                },
            );
            file_refs.push(ConversationFileRef {
                logical_path: path,
                role,
            });
        }

        resources.remove(&legacy_resource_id);
        let mut descriptor = legacy_descriptor.clone();
        descriptor.resource_id = resource_id.clone();
        resources.insert(resource_id.clone(), descriptor);
        let legacy_recipe = recipe.entries.remove(&legacy_resource_id).ok_or_else(|| {
            format!(
                "legacy conversation '{}' is absent from its recipe",
                legacy_resource_id
            )
        })?;
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: legacy_recipe.apply_policy,
                required: legacy_recipe.required,
            },
        );
        let variant = ConversationVariant {
            variant_id: variant_id.clone(),
            resource_id,
            files: file_refs,
            lineage_status,
            lineage_codec,
            head_message_id,
            message_count,
            lineage_digest,
            semantic_digest,
            parent_variant_id: None,
            fork_message_id: None,
            source_thread_id,
            created_by_origin: None,
            last_updated_by_origin: None,
            created_at: manifest.updated_at,
            updated_at: manifest.updated_at,
        };
        conversation_groups.insert(
            group_id.clone(),
            ConversationGroup {
                conflict_group_id: group_id,
                provider,
                display_title: Some(legacy_descriptor.display_name.clone()),
                primary_variant_id: variant_id.clone(),
                variants: BTreeMap::from([(variant_id, variant)]),
                created_at: manifest.updated_at,
                updated_at: manifest.updated_at,
            },
        );
    }

    Ok(PublishSchema5Request {
        identity: manifest.bundle.clone(),
        recipe,
        captured_with,
        published_by_origin,
        conversation_groups,
        resources,
        retained_files,
        captured_files,
        directories: manifest.directories.clone(),
        tombstones: manifest.tombstones.clone(),
        expected_head: PublishExpectation::Match(expected_head),
        updated_at,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct CommitRecord {
    schema_version: u32,
    bundle_id: BundleId,
    generation: u64,
    commit_id: String,
    manifest_key: String,
    manifest_sha256: String,
    created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_commit_id: Option<String>,
    added_files: u64,
    changed_files: u64,
    removed_files: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    published_by_origin: Option<PortableOrigin>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct BundleTag {
    schema_version: u32,
    bundle_id: BundleId,
    display_name: String,
    kind: BundleKind,
    generation: u64,
    updated_at: u64,
    resources: u64,
    files: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct StorageMarker {
    format: String,
    layout_version: u32,
}

impl StorageMarker {
    fn current() -> Self {
        Self {
            format: STORAGE_MARKER_FORMAT.to_string(),
            layout_version: STORAGE_LAYOUT_VERSION,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.format != STORAGE_MARKER_FORMAT {
            return Err(format!(
                "storage marker format '{}' is not supported; expected '{}'",
                self.format, STORAGE_MARKER_FORMAT
            ));
        }
        if self.layout_version != STORAGE_LAYOUT_VERSION {
            return Err(format!(
                "Mallard storage layout version {} is not supported; expected version {}",
                self.layout_version, STORAGE_LAYOUT_VERSION
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RemoteBundleSummary {
    pub bundle_id: BundleId,
    pub display_name: String,
    pub kind: BundleKind,
    pub generation: u64,
    pub updated_at: u64,
    pub resources: u64,
    pub files: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RemoteBundlePage {
    pub bundles: Vec<RemoteBundleSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub struct BundleEngine<S> {
    store: S,
    storage_id: StorageId,
}

impl<S: BundleObjectStore> BundleEngine<S> {
    pub fn open(store: S, storage_id: StorageId) -> Result<Self, String> {
        let engine = Self { store, storage_id };
        engine.ensure_storage_marker()?;
        Ok(engine)
    }

    fn ensure_storage_marker(&self) -> Result<(), String> {
        let key = ObjectKey::parse(STORAGE_MARKER_KEY)?;
        if let Some(stored) = self.store.get(&key)? {
            let marker: StorageMarker =
                parse_bounded_json(&stored.bytes, "Mallard storage marker")?;
            marker.validate()?;
            return Ok(());
        }

        let bytes = serde_json::to_vec(&StorageMarker::current())
            .map_err(|error| format!("serialize Mallard storage marker: {}", error))?;
        self.store.put_immutable(&key, &bytes)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| "Mallard storage marker disappeared after creation".to_string())?;
        let marker: StorageMarker = parse_bounded_json(&stored.bytes, "Mallard storage marker")?;
        marker.validate()
    }

    pub fn read_head(
        &self,
        bundle_id: &BundleId,
    ) -> Result<Option<(BundleHead, HeadToken)>, String> {
        let key = repository_object_key(bundle_id, HEAD_FILE)?;
        let Some(stored) = self.store.get(&key)? else {
            return Ok(None);
        };
        let head: BundleHead = parse_bounded_json(&stored.bytes, "bundle head")?;
        head.validate()?;
        if &head.bundle_id != bundle_id {
            return Err(format!(
                "bundle head '{}' is stored under bundle '{}'",
                head.bundle_id, bundle_id
            ));
        }
        let token = HeadToken {
            generation: head.generation,
            commit_id: head.commit_id.clone(),
            manifest_sha256: head.manifest_sha256.clone(),
            etag: stored.etag,
        };
        Ok(Some((head, token)))
    }

    /// Publish immutable objects and history first, then compare-and-swap the
    /// single bundle head. A failed CAS can leave only unreachable immutable
    /// objects, never a partially visible generation.
    pub fn publish(&self, request: PublishBundleRequest) -> Result<PublishedBundle, String> {
        request.identity.validate()?;
        request.recipe.validate()?;
        if request.identity.kind != BundleKind::Project {
            return Err("schema-3 v1 supports only project bundles".to_string());
        }
        if request.updated_at == 0 {
            return Err("publish timestamp must be non-zero".to_string());
        }

        let current = self.read_head(&request.identity.bundle_id)?;
        if current
            .as_ref()
            .is_some_and(|(head, _)| head.schema_version == BUNDLE_SCHEMA_V5)
        {
            return Err(
                "schema-5 bundle requires the conversation-variant publication path".to_string(),
            );
        }
        let cas_expectation = match (&request.expected_head, &current) {
            (PublishExpectation::Absent, None) => CasExpectation::Absent,
            (PublishExpectation::Absent, Some(_)) => {
                return Err("bundle head changed: expected an absent head".to_string())
            }
            (PublishExpectation::Match(expected), Some((head, token)))
                if expected == token
                    && expected.generation == head.generation
                    && expected.commit_id == head.commit_id
                    && expected.manifest_sha256 == head.manifest_sha256 =>
            {
                CasExpectation::Match(token.etag.clone())
            }
            (PublishExpectation::Match(_), _) => {
                return Err("bundle head changed before publication".to_string())
            }
        };
        let previous_manifest = current
            .as_ref()
            .map(|(head, _)| self.read_manifest(head))
            .transpose()?;
        let generation = current
            .as_ref()
            .map(|(head, _)| head.generation.saturating_add(1))
            .unwrap_or(1);
        let commit_id = random_named_id("commit")?;
        let upload_id = random_named_id("upload")?;

        let captured_descriptors = provider_capture::domain_resources(&request.captured)?;
        // Only fetch materializes dependency actions, but invalid captured
        // dependency intent must still fail the publish, not the next pull.
        provider_capture::domain_dependency_actions(&request.captured)?;
        let (resources, mut files, mut directories, tombstones) = reconcile_manifest_content(
            &request.recipe,
            &captured_descriptors,
            &request.captured,
            previous_manifest.as_ref(),
            request.updated_at,
        )?;

        for (logical_path, captured) in &request.captured.files {
            let logical = LogicalPath::parse(logical_path.clone())?;
            let resource_id = ResourceId::parse(captured.resource_id.clone())?;
            if !request.recipe.entries.contains_key(&resource_id) {
                return Err(format!(
                    "captured file '{}' belongs to unselected resource '{}'",
                    logical, resource_id
                ));
            }
            let digest = sha256(&captured.bytes);
            let relative_object_key = format!("_uploads/{}/files/{}", upload_id, logical);
            let full_object_key =
                repository_object_key(&request.identity.bundle_id, &relative_object_key)?;
            self.store
                .put_immutable(&full_object_key, &captured.bytes)?;
            files.insert(
                logical,
                BundleFileEntry {
                    resource_id,
                    sha256: digest,
                    size: captured.bytes.len() as u64,
                    source_mtime: captured.source_mtime,
                    object_key: relative_object_key,
                    mode: Some(captured.mode & 0o777),
                },
            );
        }

        for (logical_path, captured) in &request.captured.directories {
            let logical = LogicalPath::parse(logical_path.clone())?;
            let resource_id = ResourceId::parse(captured.resource_id.clone())?;
            if !request.recipe.entries.contains_key(&resource_id) {
                return Err(format!(
                    "captured directory '{}' belongs to unselected resource '{}'",
                    logical, resource_id
                ));
            }
            directories.insert(
                logical,
                BundleDirectoryEntry {
                    resource_id,
                    mode: Some(captured.mode & 0o777),
                    source_mtime: captured.source_mtime,
                },
            );
        }

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA_V4,
            generation,
            commit_id: commit_id.clone(),
            updated_at: request.updated_at,
            bundle: request.identity.clone(),
            recipe: request.recipe,
            captured_with: request.captured_with,
            published_by_origin: None,
            conversation_groups: BTreeMap::new(),
            resources,
            files,
            directories,
            tombstones,
        };
        manifest.validate()?;
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|e| format!("serialize bundle manifest: {}", e))?;
        let manifest_sha256 = sha256(&manifest_bytes);
        let manifest_relative_key = format!("_manifests/{}-{}.json", generation, commit_id);
        let manifest_key =
            repository_object_key(&request.identity.bundle_id, &manifest_relative_key)?;
        self.store.put_immutable(&manifest_key, &manifest_bytes)?;

        let (added_files, changed_files, removed_files) =
            file_delta(previous_manifest.as_ref(), &manifest);
        let commit = CommitRecord {
            schema_version: BUNDLE_SCHEMA_V4,
            bundle_id: request.identity.bundle_id.clone(),
            generation,
            commit_id: commit_id.clone(),
            manifest_key: manifest_relative_key.clone(),
            manifest_sha256: manifest_sha256.clone(),
            created_at: request.updated_at,
            previous_commit_id: current.as_ref().map(|(head, _)| head.commit_id.clone()),
            added_files,
            changed_files,
            removed_files,
            published_by_origin: None,
        };
        let commit_bytes =
            serde_json::to_vec(&commit).map_err(|e| format!("serialize bundle commit: {}", e))?;
        let commit_key = repository_object_key(
            &request.identity.bundle_id,
            &format!("_commits/{}-{}.json", generation, commit_id),
        )?;
        self.store.put_immutable(&commit_key, &commit_bytes)?;

        let head = BundleHead {
            schema_version: BUNDLE_SCHEMA_V4,
            bundle_id: request.identity.bundle_id.clone(),
            kind: request.identity.kind,
            generation,
            commit_id,
            manifest_key: manifest_relative_key,
            manifest_sha256,
            updated_at: request.updated_at,
        };
        head.validate()?;
        let head_bytes =
            serde_json::to_vec(&head).map_err(|e| format!("serialize bundle head: {}", e))?;
        let head_key = repository_object_key(&request.identity.bundle_id, HEAD_FILE)?;
        let head_etag =
            match self
                .store
                .compare_and_swap(&head_key, &cas_expectation, &head_bytes)?
            {
                CasOutcome::Written { etag } => etag,
                CasOutcome::Conflict { .. } => {
                    return Err(
                        "bundle head CAS conflict; fetch and rebase before retrying".to_string()
                    )
                }
            };
        // The tag is a derived discovery hint, never a second authority. A
        // head that was successfully CAS-published remains a successful
        // publish even if a contended tag update must be repaired later.
        let _ = self.publish_tag(&manifest);

        let snapshot = BundleSnapshot {
            storage_id: self.storage_id.clone(),
            head: head.clone(),
            manifest,
            fetched_at: request.updated_at,
        };
        Ok(PublishedBundle {
            snapshot,
            head_token: HeadToken {
                generation: head.generation,
                commit_id: head.commit_id.clone(),
                manifest_sha256: head.manifest_sha256.clone(),
                etag: head_etag,
            },
        })
    }

    /// Publish a complete schema-5 conversation snapshot. The caller supplies
    /// the reviewed live set; the engine enforces remote-variant retention,
    /// verifies retained immutable objects, skips true no-ops, and advances
    /// the head only through compare-and-swap.
    pub fn publish_schema5(
        &self,
        request: PublishSchema5Request,
    ) -> Result<Schema5PublishOutcome, String> {
        request.identity.validate()?;
        request.recipe.validate()?;
        request.published_by_origin.validate()?;
        if request.identity.kind != BundleKind::Project {
            return Err("schema-5 supports only project bundles".to_string());
        }
        if request.updated_at == 0 {
            return Err("publish timestamp must be non-zero".to_string());
        }
        for path in request.captured_files.keys() {
            if request.retained_files.contains_key(path) {
                return Err(format!(
                    "schema-5 file '{}' is both retained and captured",
                    path
                ));
            }
        }

        let current = self.read_head(&request.identity.bundle_id)?;
        let cas_expectation = match (&request.expected_head, &current) {
            (PublishExpectation::Absent, None) => CasExpectation::Absent,
            (PublishExpectation::Absent, Some(_)) => {
                return Err("bundle head changed: expected an absent head".to_string())
            }
            (PublishExpectation::Match(expected), Some((head, token)))
                if expected == token
                    && expected.generation == head.generation
                    && expected.commit_id == head.commit_id
                    && expected.manifest_sha256 == head.manifest_sha256 =>
            {
                CasExpectation::Match(token.etag.clone())
            }
            (PublishExpectation::Match(_), _) => {
                return Err("bundle head changed before publication".to_string())
            }
        };
        let previous_manifest = current
            .as_ref()
            .map(|(head, _)| self.read_manifest(head))
            .transpose()?;

        for (path, entry) in &request.retained_files {
            entry.validate()?;
            let previous = previous_manifest.as_ref().ok_or_else(|| {
                format!("schema-5 retained file '{}' has no previous manifest", path)
            })?;
            let was_published = previous.files.values().any(|candidate| {
                candidate.object_key == entry.object_key
                    && candidate.sha256 == entry.sha256
                    && candidate.size == entry.size
            });
            if !was_published {
                return Err(format!(
                    "schema-5 retained file '{}' was not in the reviewed manifest",
                    path
                ));
            }
            let key = repository_object_key(&request.identity.bundle_id, &entry.object_key)?;
            let object = self
                .store
                .get(&key)?
                .ok_or_else(|| format!("retained bundle object '{}' is missing", key))?;
            if object.bytes.len() as u64 != entry.size || sha256(&object.bytes) != entry.sha256 {
                return Err(format!(
                    "retained bundle object '{}' does not match its reviewed digest",
                    key
                ));
            }
        }

        let generation = current
            .as_ref()
            .map(|(head, _)| head.generation.saturating_add(1))
            .unwrap_or(1);
        let commit_id = random_named_id("commit")?;
        let upload_id = random_named_id("upload")?;
        let mut files = request.retained_files.clone();
        for (path, captured) in &request.captured_files {
            if captured.bytes.len() as u64 > domain::MAX_FILE_BYTES {
                return Err(format!(
                    "schema-5 file '{}' exceeds the portable limit",
                    path
                ));
            }
            if captured.mode.is_some_and(|mode| mode & !0o777 != 0) {
                return Err(format!("schema-5 file '{}' has unsafe mode bits", path));
            }
            if !request.resources.contains_key(&captured.resource_id) {
                return Err(format!(
                    "schema-5 file '{}' belongs to missing resource '{}'",
                    path, captured.resource_id
                ));
            }
            files.insert(
                path.clone(),
                BundleFileEntry {
                    resource_id: captured.resource_id.clone(),
                    sha256: sha256(&captured.bytes),
                    size: captured.bytes.len() as u64,
                    source_mtime: captured.source_mtime,
                    object_key: format!("_uploads/{}/files/{}", upload_id, path),
                    mode: captured.mode,
                },
            );
        }

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA_V5,
            generation,
            commit_id: commit_id.clone(),
            updated_at: request.updated_at,
            bundle: request.identity.clone(),
            recipe: request.recipe,
            captured_with: request.captured_with,
            published_by_origin: Some(request.published_by_origin.clone()),
            conversation_groups: request.conversation_groups,
            resources: request.resources,
            files,
            directories: request.directories,
            tombstones: request.tombstones,
        };
        manifest.validate()?;
        if let Some(previous) = &previous_manifest {
            validate_schema5_conversation_retention(previous, &manifest)?;
        }

        if let (Some((head, token)), Some(previous)) = (&current, &previous_manifest) {
            if previous.schema_version == BUNDLE_SCHEMA_V5
                && schema5_manifest_is_semantic_noop(previous, &manifest)
            {
                return Ok(Schema5PublishOutcome {
                    published: false,
                    snapshot: BundleSnapshot {
                        storage_id: self.storage_id.clone(),
                        head: head.clone(),
                        manifest: previous.clone(),
                        fetched_at: request.updated_at,
                    },
                    head_token: token.clone(),
                });
            }
        }

        for (path, captured) in &request.captured_files {
            let entry = manifest
                .files
                .get(path)
                .expect("captured schema-5 file was inserted");
            let key = repository_object_key(&request.identity.bundle_id, &entry.object_key)?;
            self.store.put_immutable(&key, &captured.bytes)?;
            let stored = self
                .store
                .get(&key)?
                .ok_or_else(|| format!("schema-5 object '{}' disappeared", key))?;
            if stored.bytes.len() as u64 != entry.size || sha256(&stored.bytes) != entry.sha256 {
                return Err(format!(
                    "schema-5 object '{}' failed post-publication verification",
                    key
                ));
            }
        }

        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| format!("serialize schema-5 manifest: {error}"))?;
        let manifest_sha256 = sha256(&manifest_bytes);
        let manifest_relative_key = format!("_manifests/{}-{}.json", generation, commit_id);
        let manifest_key =
            repository_object_key(&request.identity.bundle_id, &manifest_relative_key)?;
        self.store.put_immutable(&manifest_key, &manifest_bytes)?;

        let (added_files, changed_files, removed_files) =
            file_delta(previous_manifest.as_ref(), &manifest);
        let commit = CommitRecord {
            schema_version: BUNDLE_SCHEMA_V5,
            bundle_id: request.identity.bundle_id.clone(),
            generation,
            commit_id: commit_id.clone(),
            manifest_key: manifest_relative_key.clone(),
            manifest_sha256: manifest_sha256.clone(),
            created_at: request.updated_at,
            previous_commit_id: current.as_ref().map(|(head, _)| head.commit_id.clone()),
            added_files,
            changed_files,
            removed_files,
            published_by_origin: Some(request.published_by_origin),
        };
        let commit_bytes = serde_json::to_vec(&commit)
            .map_err(|error| format!("serialize schema-5 commit: {error}"))?;
        let commit_key = repository_object_key(
            &request.identity.bundle_id,
            &format!("_commits/{}-{}.json", generation, commit_id),
        )?;
        self.store.put_immutable(&commit_key, &commit_bytes)?;

        let head = BundleHead {
            schema_version: BUNDLE_SCHEMA_V5,
            bundle_id: request.identity.bundle_id.clone(),
            kind: request.identity.kind,
            generation,
            commit_id,
            manifest_key: manifest_relative_key,
            manifest_sha256,
            updated_at: request.updated_at,
        };
        head.validate()?;
        let head_bytes = serde_json::to_vec(&head)
            .map_err(|error| format!("serialize schema-5 head: {error}"))?;
        let head_key = repository_object_key(&request.identity.bundle_id, HEAD_FILE)?;
        let head_etag =
            match self
                .store
                .compare_and_swap(&head_key, &cas_expectation, &head_bytes)?
            {
                CasOutcome::Written { etag } => etag,
                CasOutcome::Conflict { .. } => {
                    return Err(
                        "bundle head CAS conflict; fetch and rebase before retrying".to_string()
                    )
                }
            };
        let _ = self.publish_tag(&manifest);
        Ok(Schema5PublishOutcome {
            published: true,
            snapshot: BundleSnapshot {
                storage_id: self.storage_id.clone(),
                head: head.clone(),
                manifest,
                fetched_at: request.updated_at,
            },
            head_token: HeadToken {
                generation: head.generation,
                commit_id: head.commit_id.clone(),
                manifest_sha256: head.manifest_sha256.clone(),
                etag: head_etag,
            },
        })
    }

    pub fn fetch(&self, bundle_id: &BundleId) -> Result<FetchedBundle, String> {
        let snapshot = self.inspect(bundle_id)?;
        let mut files = BTreeMap::new();
        for (logical_path, entry) in &snapshot.manifest.files {
            let key = repository_object_key(bundle_id, &entry.object_key)?;
            let object = self
                .store
                .get(&key)?
                .ok_or_else(|| format!("bundle object '{}' is missing", key))?;
            if object.bytes.len() as u64 != entry.size || sha256(&object.bytes) != entry.sha256 {
                return Err(format!(
                    "bundle object '{}' does not match manifest size/hash",
                    key
                ));
            }
            files.insert(logical_path.clone(), object.bytes);
        }
        let dependency_actions = dependencies_from_manifest(&snapshot.manifest)?;
        Ok(FetchedBundle {
            snapshot,
            files,
            dependency_actions,
        })
    }

    /// Verify a bundle's mutable head and immutable manifest without loading
    /// payload objects. Discovery uses this to match repository identities
    /// without downloading every session or project file in a storage.
    pub fn inspect(&self, bundle_id: &BundleId) -> Result<BundleSnapshot, String> {
        self.inspect_optional(bundle_id)?
            .ok_or_else(|| format!("bundle '{}' does not exist", bundle_id))
    }

    /// The optional form is used by read-only status views where an empty
    /// linked destination is a valid state rather than an error.
    pub fn inspect_optional(&self, bundle_id: &BundleId) -> Result<Option<BundleSnapshot>, String> {
        let Some((head, _)) = self.read_head(bundle_id)? else {
            return Ok(None);
        };
        let manifest = self.read_manifest(&head)?;
        Ok(Some(BundleSnapshot {
            storage_id: self.storage_id.clone(),
            head,
            manifest,
            fetched_at: now_secs(),
        }))
    }

    /// Load one immutable historical manifest using the coordinates retained
    /// by a reviewed local sync base. This reads metadata only; payload
    /// objects such as conversation rollouts are not downloaded.
    pub fn inspect_manifest_version(
        &self,
        bundle_id: &BundleId,
        generation: u64,
        commit_id: &str,
        manifest_sha256: &str,
    ) -> Result<BundleManifest, String> {
        domain::validate_sha256("historical manifest", manifest_sha256)?;
        let relative_key = format!("_manifests/{generation}-{commit_id}.json");
        validate_bundle_relative_object_key(&relative_key)?;
        let key = repository_object_key(bundle_id, &relative_key)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| format!("bundle manifest '{}' is missing", key))?;
        if sha256(&stored.bytes) != manifest_sha256 {
            return Err(format!(
                "bundle manifest '{}' failed hash verification",
                key
            ));
        }
        let manifest: BundleManifest = parse_bounded_json(&stored.bytes, "bundle manifest")?;
        manifest.validate()?;
        if &manifest.bundle.bundle_id != bundle_id
            || manifest.generation != generation
            || manifest.commit_id != commit_id
        {
            return Err("historical manifest coordinates do not match its contents".to_string());
        }
        Ok(manifest)
    }

    /// Cursor-paginated remote discovery. The cursor is the last returned
    /// bundle ID, not a mutable global catalog offset.
    pub fn list_remote_bundles(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RemoteBundlePage, String> {
        if limit == 0 || limit > 500 {
            return Err("bundle page limit must be between 1 and 500".to_string());
        }
        let after = cursor.map(BundleId::parse).transpose()?;
        let prefix = ObjectPrefix::parse(REPOSITORY_PREFIX)?;
        let mut object_cursor = None;
        let mut tag_keys = Vec::new();
        let mut observed = 0_usize;
        loop {
            let page = self
                .store
                .list(&prefix, object_cursor.as_deref(), MAX_LIST_PAGE)?;
            observed = observed.saturating_add(page.keys.len());
            if observed > 1_000_000 {
                return Err("remote bundle listing exceeds one million objects".to_string());
            }
            tag_keys.extend(
                page.keys
                    .into_iter()
                    .filter(|key| key.as_str().ends_with("/_tag.json")),
            );
            let Some(next) = page.next_cursor else {
                break;
            };
            object_cursor = Some(next);
        }

        let mut summaries = Vec::new();
        for key in tag_keys {
            let Some(bundle_component) = key
                .as_str()
                .strip_prefix(&format!("{}/", REPOSITORY_PREFIX))
                .and_then(|rest| rest.strip_suffix("/_tag.json"))
            else {
                continue;
            };
            let bundle_id = BundleId::parse(bundle_component.to_string())?;
            if after.as_ref().is_some_and(|after| &bundle_id <= after) {
                continue;
            }
            let stored = self
                .store
                .get(&key)?
                .ok_or_else(|| format!("listed bundle tag '{}' disappeared", key))?;
            let tag: BundleTag = parse_bounded_json(&stored.bytes, "bundle tag")?;
            if !matches!(tag.schema_version, BUNDLE_SCHEMA_V4 | BUNDLE_SCHEMA_V5)
                || tag.bundle_id != bundle_id
            {
                return Err(format!("bundle tag '{}' has inconsistent identity", key));
            }
            summaries.push(RemoteBundleSummary {
                bundle_id: tag.bundle_id,
                display_name: tag.display_name,
                kind: tag.kind,
                generation: tag.generation,
                updated_at: tag.updated_at,
                resources: tag.resources,
                files: tag.files,
            });
        }
        summaries.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
        let has_more = summaries.len() > limit;
        summaries.truncate(limit);
        let next_cursor = has_more.then(|| summaries.last().unwrap().bundle_id.to_string());
        Ok(RemoteBundlePage {
            bundles: summaries,
            next_cursor,
        })
    }

    pub fn build_restore_plan(
        &self,
        bundle: &FetchedBundle,
        binding: &ProjectBinding,
        created_at: u64,
        expected_host_identity_revision: u64,
    ) -> Result<RestorePlan, String> {
        validate_bundle_snapshot_bytes(bundle)?;
        self.validate_binding(binding, None)?;
        if binding.state != BindingState::Active {
            return Err("restore requires an active project binding".to_string());
        }
        if binding.bundle_id != bundle.snapshot.head.bundle_id {
            return Err("binding and fetched bundle IDs differ".to_string());
        }
        if bundle.snapshot.storage_id != self.storage_id {
            return Err("fetched bundle belongs to another storage".to_string());
        }
        let plan_id = PlanId::parse(random_named_id("plan")?)?;
        let mut actions = Vec::new();
        let mut target_owners = BTreeMap::<String, LogicalPath>::new();
        let mut continuation_ids = BTreeSet::new();

        // Executable dependencies have their own immutable DependencyPlan.
        // Keeping plugin/installer placeholders out of RestorePlan prevents a
        // single logical resource from appearing twice in Pull review and
        // prevents the restore engine from reporting expected native work as
        // a blocked file action.
        // Custom skills are directory units: one typed install/overwrite
        // action per resource, never independently selectable file writes.
        let mut skill_targets = BTreeMap::<String, ResourceId>::new();
        let mut skill_capabilities = BTreeMap::<String, ResourceId>::new();
        for descriptor in bundle.snapshot.manifest.resources.values() {
            if descriptor.kind != ResourceKind::StandaloneSkill {
                continue;
            }
            let identity = custom_skill_identity(descriptor)?;
            let capability_key = format!(
                "{:?}:{}",
                identity.provider,
                identity.effective_name.to_ascii_lowercase()
            );
            if let Some(previous) =
                skill_capabilities.insert(capability_key, descriptor.resource_id.clone())
            {
                return Err(format!(
                    "custom skills '{}' and '{}' claim one effective name",
                    previous, descriptor.resource_id
                ));
            }
            let action = custom_skill_action(bundle, descriptor, binding)?;
            if let Some(target) = &action.target_path {
                // Case-folded so two snapshots cannot claim one materialized
                // directory on a case-insensitive filesystem.
                if let Some(previous) =
                    skill_targets.insert(target.to_lowercase(), descriptor.resource_id.clone())
                {
                    return Err(format!(
                        "custom skills '{}' and '{}' claim one target directory",
                        previous, descriptor.resource_id
                    ));
                }
            }
            actions.push(action);
        }
        for (logical_path, entry) in &bundle.snapshot.manifest.directories {
            let descriptor = bundle
                .snapshot
                .manifest
                .resources
                .get(&entry.resource_id)
                .ok_or_else(|| format!("directory '{}' has no resource", logical_path))?;
            if descriptor.kind != ResourceKind::ProjectContentDirectory {
                return Err(format!(
                    "directory '{}' is not project-content metadata",
                    logical_path
                ));
            }
            let target = project_content_target(logical_path, binding)?;
            let expected_target_mode = match inspect_project_directory_target(&target) {
                Ok(mode) => mode,
                Err(error) => {
                    actions.push(manual_action(
                        &entry.resource_id,
                        &format!(
                            "Project directory '{}' is blocked at '{}': {}",
                            logical_path,
                            target.display(),
                            error
                        ),
                    )?);
                    continue;
                }
            };
            let mode = entry.mode.unwrap_or(0o700) & 0o777;
            let kind = RestoreActionKind::EnsureProjectDirectory {
                logical_path: logical_path.clone(),
                mode,
                source_mtime: entry.source_mtime,
            };
            actions.push(RestoreAction {
                action_id: action_id_for(
                    &entry.resource_id,
                    logical_path.as_str(),
                    kind.action_type(),
                )?,
                resource_id: entry.resource_id.clone(),
                kind,
                target_path: Some(path_text(&target)?),
                source_sha256: Some(directory_entry_digest(logical_path, mode)),
                expected_target_sha256: None,
                expected_target_mode,
                requires_explicit_approval: true,
            });
        }
        for (logical_path, entry) in &bundle.snapshot.manifest.files {
            let descriptor = bundle
                .snapshot
                .manifest
                .resources
                .get(&entry.resource_id)
                .ok_or_else(|| format!("file '{}' has no resource", logical_path))?;
            if descriptor.kind == ResourceKind::StandaloneSkill {
                continue;
            }
            let source_bytes = bundle
                .files
                .get(logical_path)
                .ok_or_else(|| format!("fetched bytes for '{}' are missing", logical_path))?;
            let materialized_bytes =
                materialized_file_bytes(logical_path, descriptor, binding, source_bytes)?;
            let materialized_sha256 = sha256(&materialized_bytes);
            let Some(target) = map_logical_target(logical_path, descriptor, binding)? else {
                actions.push(manual_action(
                    &entry.resource_id,
                    &format!("No safe materializer is available for {}", logical_path),
                )?);
                continue;
            };
            let folded = target.to_string_lossy().to_lowercase();
            if let Some(previous) = target_owners.insert(folded, logical_path.clone()) {
                return Err(format!(
                    "logical files '{}' and '{}' map to one target",
                    previous, logical_path
                ));
            }
            let target_state = match inspect_restore_target(&target) {
                Ok(state) => state,
                Err(error) if descriptor.kind == ResourceKind::ProjectContentFile => {
                    actions.push(manual_action(
                        &entry.resource_id,
                        &format!(
                            "Project file '{}' is blocked at '{}': {}",
                            logical_path,
                            target.display(),
                            error
                        ),
                    )?);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if logical_path.as_str() != "state/codex/session_index.jsonl"
                && matches!(
                    descriptor.kind,
                    ResourceKind::CodexConversation | ResourceKind::ClaudeConversation
                )
                && target_state
                    .digest
                    .as_ref()
                    .is_some_and(|digest| digest != &materialized_sha256 && digest != &entry.sha256)
            {
                actions.push(conversation_keep_local_action(
                    &entry.resource_id,
                    logical_path,
                    &target,
                    &entry.sha256,
                    target_state.digest.as_deref().unwrap(),
                )?);
                continue;
            }

            let kind = restore_kind_for_file(logical_path, descriptor, entry)?;
            // A conversation copied by an older Agent Sync release can still
            // have the exact portable source digest. Treat it as the same
            // known session so the path-only migration stays a safe default.
            let approval_target_digest = target_state.digest.as_deref().map(|digest| {
                if matches!(
                    descriptor.kind,
                    ResourceKind::CodexConversation | ResourceKind::ClaudeConversation
                ) && digest == entry.sha256
                {
                    materialized_sha256.as_str()
                } else {
                    digest
                }
            });
            let requires_explicit_approval = action_needs_explicit_approval(
                descriptor,
                approval_target_digest,
                &materialized_sha256,
            );
            actions.push(RestoreAction {
                action_id: action_id_for(
                    &entry.resource_id,
                    logical_path.as_str(),
                    kind.action_type(),
                )?,
                resource_id: entry.resource_id.clone(),
                kind,
                target_path: Some(path_text(&target)?),
                source_sha256: Some(entry.sha256.clone()),
                expected_target_sha256: target_state.digest,
                expected_target_mode: (descriptor.kind == ResourceKind::ProjectContentFile)
                    .then_some(target_state.mode)
                    .flatten(),
                requires_explicit_approval,
            });
            if descriptor.relative_cwd.is_some()
                && matches!(descriptor.kind, ResourceKind::ClaudeConversation)
                && continuation_ids.insert(descriptor.resource_id.clone())
            {
                actions.push(continuation_action(descriptor, binding)?);
            }
        }
        for tombstone in bundle.snapshot.manifest.tombstones.values() {
            match &tombstone.target {
                TombstoneTarget::ProjectContentFile {
                    resource_id,
                    logical_path,
                } => {
                    let last_sha256 = tombstone.last_sha256.clone().ok_or_else(|| {
                        format!(
                            "project-content file tombstone '{}' lacks its last digest",
                            logical_path
                        )
                    })?;
                    let target = project_content_target(logical_path, binding)?;
                    let target_state = match inspect_restore_target(&target) {
                        Ok(state) => state,
                        Err(error) => {
                            actions.push(manual_action(
                                resource_id,
                                &format!(
                                    "Project-file deletion '{}' is blocked at '{}': {}",
                                    logical_path,
                                    target.display(),
                                    error
                                ),
                            )?);
                            continue;
                        }
                    };
                    if target_state.digest.is_none() {
                        continue;
                    }
                    let kind = RestoreActionKind::DeleteProjectFile {
                        logical_path: logical_path.clone(),
                        last_sha256: last_sha256.clone(),
                    };
                    actions.push(RestoreAction {
                        action_id: action_id_for(
                            resource_id,
                            logical_path.as_str(),
                            kind.action_type(),
                        )?,
                        resource_id: resource_id.clone(),
                        kind,
                        target_path: Some(path_text(&target)?),
                        source_sha256: None,
                        expected_target_sha256: Some(last_sha256),
                        expected_target_mode: target_state.mode,
                        requires_explicit_approval: true,
                    });
                }
                TombstoneTarget::ProjectContentDirectory {
                    resource_id,
                    logical_path,
                } => {
                    let target = project_content_target(logical_path, binding)?;
                    let expected_target_mode = match inspect_project_directory_target(&target) {
                        Ok(Some(mode)) => Some(mode),
                        Ok(None) => continue,
                        Err(error) => {
                            actions.push(manual_action(
                                resource_id,
                                &format!(
                                    "Project-directory deletion '{}' is blocked at '{}': {}",
                                    logical_path,
                                    target.display(),
                                    error
                                ),
                            )?);
                            continue;
                        }
                    };
                    let kind = RestoreActionKind::DeleteProjectDirectory {
                        logical_path: logical_path.clone(),
                    };
                    actions.push(RestoreAction {
                        action_id: action_id_for(
                            resource_id,
                            logical_path.as_str(),
                            kind.action_type(),
                        )?,
                        resource_id: resource_id.clone(),
                        kind,
                        target_path: Some(path_text(&target)?),
                        source_sha256: None,
                        expected_target_sha256: None,
                        expected_target_mode,
                        requires_explicit_approval: true,
                    });
                }
                _ => {}
            }
        }
        actions.sort_by(|a, b| a.action_id.cmp(&b.action_id));
        let plan = RestorePlan {
            schema_version: RESTORE_PLAN_SCHEMA_V2,
            plan_id,
            storage_id: self.storage_id.clone(),
            bundle_id: bundle.snapshot.head.bundle_id.clone(),
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            expected_host_identity_revision,
            generation: bundle.snapshot.head.generation,
            commit_id: bundle.snapshot.head.commit_id.clone(),
            manifest_sha256: bundle.snapshot.head.manifest_sha256.clone(),
            binding_revision: binding.revision,
            created_at,
            expires_at: created_at.saturating_add(DEFAULT_PLAN_LIFETIME_SECS),
            actions,
            conversation_actions: Vec::new(),
            project_content_eligibility: None,
        };
        plan.validate()?;
        Ok(plan)
    }

    fn read_manifest(&self, head: &BundleHead) -> Result<BundleManifest, String> {
        head.validate()?;
        validate_bundle_relative_object_key(&head.manifest_key)?;
        let key = repository_object_key(&head.bundle_id, &head.manifest_key)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| format!("bundle manifest '{}' is missing", key))?;
        if sha256(&stored.bytes) != head.manifest_sha256 {
            return Err(format!(
                "bundle manifest '{}' failed hash verification",
                key
            ));
        }
        let manifest: BundleManifest = parse_bounded_json(&stored.bytes, "bundle manifest")?;
        manifest.validate_against_head(head)?;
        Ok(manifest)
    }

    fn publish_tag(&self, manifest: &BundleManifest) -> Result<(), String> {
        let tag = BundleTag {
            schema_version: manifest.schema_version,
            bundle_id: manifest.bundle.bundle_id.clone(),
            display_name: manifest.bundle.display_name.clone(),
            kind: manifest.bundle.kind,
            generation: manifest.generation,
            updated_at: manifest.updated_at,
            resources: manifest.resources.len() as u64,
            files: manifest.files.len() as u64,
        };
        let bytes = serde_json::to_vec(&tag).map_err(|e| format!("serialize bundle tag: {}", e))?;
        let key = repository_object_key(&manifest.bundle.bundle_id, TAG_FILE)?;
        for _ in 0..4 {
            let expectation = match self.store.get(&key)? {
                Some(existing) => CasExpectation::Match(existing.etag),
                None => CasExpectation::Absent,
            };
            if matches!(
                self.store.compare_and_swap(&key, &expectation, &bytes)?,
                CasOutcome::Written { .. }
            ) {
                return Ok(());
            }
        }
        Err("bundle tag CAS remained contended after publication".to_string())
    }

    fn validate_binding(
        &self,
        binding: &ProjectBinding,
        backup_root: Option<&Path>,
    ) -> Result<(), String> {
        binding.validate_structure()?;
        let project = canonical_real_dir(Path::new(&binding.project_root), "project root")?;
        let recorded = PathBuf::from(&binding.canonical_project_root);
        if project != recorded {
            return Err(format!(
                "project binding changed: '{}' now resolves to '{}'",
                binding.project_root,
                project.display()
            ));
        }
        let mut targets = vec![project];
        for (label, home) in [
            ("Codex home", binding.codex_home.as_deref()),
            ("Claude home", binding.claude_home.as_deref()),
        ] {
            if let Some(home) = home {
                targets.push(canonical_real_dir(Path::new(home), label)?);
            }
        }
        if let Some(store_root) = self.store.local_root() {
            for target in &targets {
                if paths_overlap(store_root, target) {
                    return Err(format!(
                        "local bundle store '{}' overlaps restore target '{}'",
                        store_root.display(),
                        target.display()
                    ));
                }
            }
        }
        if let Some(backup_root) = backup_root {
            let backup = canonical_or_prospective_dir(backup_root, "backup root")?;
            for target in &targets {
                if paths_overlap(&backup, target) {
                    return Err(format!(
                        "backup root '{}' overlaps restore target '{}'",
                        backup.display(),
                        target.display()
                    ));
                }
            }
            if self
                .store
                .local_root()
                .is_some_and(|store| paths_overlap(store, &backup))
            {
                return Err("backup root overlaps local bundle storage".to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_sync_v3::domain::{
        LocalProjectId, RecipeEntry, ReplicaId, RestoreActionType,
    };
    use crate::project_sync_v3::global_inventory::GlobalPluginSource;
    use crate::project_sync_v3::provider_capture::{
        capture_recipe, capture_selected, discover_project, CaptureRequest, CaptureResourceKind,
        Provider as CaptureProvider,
    };

    fn bundle_id(number: u64) -> BundleId {
        BundleId::parse(format!("{:032x}", number)).unwrap()
    }

    fn storage_id() -> StorageId {
        StorageId::parse("storage-local").unwrap()
    }

    fn identity(id: BundleId, name: &str) -> BundleIdentity {
        BundleIdentity {
            bundle_id: id,
            display_name: name.to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        }
    }

    fn recipe_for(resource_ids: impl IntoIterator<Item = String>) -> BundleRecipe {
        let mut recipe = BundleRecipe::default();
        for resource_id in resource_ids {
            let resource_id = ResourceId::parse(resource_id).unwrap();
            recipe.entries.insert(
                resource_id.clone(),
                RecipeEntry {
                    resource_id,
                    apply_policy: ApplyPolicy::Merge,
                    required: false,
                },
            );
        }
        recipe
    }

    fn captured_with() -> CapturedWith {
        CapturedWith {
            app_version: "test".to_string(),
            codex_version: None,
            claude_version: None,
            codec_versions: BTreeMap::from([("codex".to_string(), 1), ("claude".to_string(), 1)]),
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn active_binding(
        id: BundleId,
        project_root: &Path,
        codex_home: Option<&Path>,
        claude_home: Option<&Path>,
    ) -> ProjectBinding {
        ProjectBinding {
            host_id: super::domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
            replica_id: ReplicaId::parse("replica-test").unwrap(),
            local_project_id: LocalProjectId::parse("project-test").unwrap(),
            bundle_id: id,
            project_root: project_root.to_str().unwrap().to_string(),
            canonical_project_root: fs::canonicalize(project_root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            profile_ids: BTreeMap::new(),
            codex_home: codex_home.map(|path| path.to_str().unwrap().to_string()),
            claude_home: claude_home.map(|path| path.to_str().unwrap().to_string()),
            state: BindingState::Active,
            revision: 3,
            updated_at: 1,
        }
    }

    #[test]
    fn codex_session_index_merge_is_approved_with_conversations_by_default() {
        let index = ResourceDescriptor {
            resource_id: ResourceId::parse("codex:session-index").unwrap(),
            kind: ResourceKind::CodexConversation,
            provider: Some(Provider::Codex),
            scope: domain::ResourceScope::ProviderState,
            display_name: "Codex project session index".to_string(),
            provenance: domain::Provenance::Unknown,
            apply_policy: ApplyPolicy::Merge,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::new(),
        };
        assert!(!action_needs_explicit_approval(
            &index,
            Some("target-differs"),
            "incoming"
        ));

        let mut ordinary_merge = index;
        ordinary_merge.resource_id = ResourceId::parse("project:file:guidance").unwrap();
        ordinary_merge.kind = ResourceKind::ProjectFile;
        assert!(action_needs_explicit_approval(
            &ordinary_merge,
            Some("target-differs"),
            "incoming"
        ));
    }

    #[test]
    fn local_store_enforces_immutable_objects_and_head_cas() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalBundleObjectStore::open(temp.path().join("store")).unwrap();
        let key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{}/_uploads/upload-a/files/project/AGENTS.md",
            bundle_id(1)
        ))
        .unwrap();
        assert_eq!(
            store.put_immutable(&key, b"one").unwrap(),
            ImmutablePutOutcome::Written
        );
        assert_eq!(
            store.put_immutable(&key, b"one").unwrap(),
            ImmutablePutOutcome::AlreadyPresent
        );
        assert!(store.put_immutable(&key, b"two").is_err());

        let head = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{}/_head.json",
            bundle_id(1)
        ))
        .unwrap();
        let first = store
            .compare_and_swap(&head, &CasExpectation::Absent, b"first")
            .unwrap();
        let CasOutcome::Written { etag } = first else {
            panic!("initial CAS did not write")
        };
        assert!(matches!(
            store
                .compare_and_swap(&head, &CasExpectation::Absent, b"lost")
                .unwrap(),
            CasOutcome::Conflict { .. }
        ));
        assert!(matches!(
            store
                .compare_and_swap(&head, &CasExpectation::Match(etag), b"second")
                .unwrap(),
            CasOutcome::Written { .. }
        ));
        assert_eq!(store.get(&head).unwrap().unwrap().bytes, b"second");
    }

    #[test]
    fn mallard_storage_marker_is_created_idempotently_and_validated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("store");
        let store = LocalBundleObjectStore::open(&root).unwrap();
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        drop(engine);

        let marker_path = root.join(STORAGE_MARKER_KEY);
        let marker: StorageMarker =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        assert_eq!(marker, StorageMarker::current());
        assert!(root.join(MALLARD_ROOT).join(LOCAL_STORAGE_LOCK).is_file());

        let store = LocalBundleObjectStore::open(&root).unwrap();
        BundleEngine::open(store, storage_id()).unwrap();

        fs::write(
            &marker_path,
            br#"{"format":"mallard-storage","layout_version":2}"#,
        )
        .unwrap();
        let store = LocalBundleObjectStore::open(&root).unwrap();
        let error = match BundleEngine::open(store, storage_id()) {
            Ok(_) => panic!("unsupported marker should be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("layout version 2"));
    }

    #[test]
    fn legacy_bundle_namespace_is_ignored_without_migration() {
        assert!(ObjectKey::parse("v3/bundles/0123456789abcdef0123456789abcdef/_tag.json").is_err());

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("store");
        write(
            &root.join("v3/bundles/0123456789abcdef0123456789abcdef/_tag.json"),
            b"legacy",
        );
        let store = LocalBundleObjectStore::open(&root).unwrap();
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let page = engine.list_remote_bundles(None, 100).unwrap();
        assert!(page.bundles.is_empty());
        assert!(root
            .join("v3/bundles/0123456789abcdef0123456789abcdef/_tag.json")
            .is_file());
        assert!(root.join(STORAGE_MARKER_KEY).is_file());
    }

    #[test]
    fn publish_fetch_update_and_tombstone_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"version one");
        let request = CaptureRequest::for_project(&project);
        let inventory = discover_project(&request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let captured = capture_recipe(&request, &recipe).unwrap();
        let id = bundle_id(2);
        let store_root = temp.path().join("store");
        let store = LocalBundleObjectStore::open(&store_root).unwrap();
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let first = engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured,
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        assert_eq!(first.snapshot.head.generation, 1);
        let repository_root = store_root.join(format!("{}/{}", REPOSITORY_PREFIX, id));
        assert!(store_root.join(STORAGE_MARKER_KEY).is_file());
        assert!(repository_root.join(HEAD_FILE).is_file());
        assert!(repository_root.join(TAG_FILE).is_file());
        assert!(repository_root.join("_manifests").is_dir());
        assert!(repository_root.join("_commits").is_dir());
        assert!(repository_root.join("_uploads").is_dir());
        let fetched = engine.fetch(&id).unwrap();
        assert_eq!(fetched.files.values().next().unwrap(), b"version one");

        write(&project.join("CLAUDE.md"), b"version two");
        let captured = capture_recipe(&request, &recipe).unwrap();
        let second = engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Renamed Project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured,
                expected_head: PublishExpectation::Match(first.head_token.clone()),
                updated_at: 20,
            })
            .unwrap();
        assert_eq!(second.snapshot.head.generation, 2);
        assert!(engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Stale"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Match(first.head_token),
                updated_at: 21,
            })
            .is_err());

        let empty_recipe = BundleRecipe::default();
        let removed = engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Renamed Project"),
                recipe: empty_recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &empty_recipe).unwrap(),
                expected_head: PublishExpectation::Match(second.head_token),
                updated_at: 30,
            })
            .unwrap();
        assert!(removed.snapshot.manifest.files.is_empty());
        assert!(removed.snapshot.manifest.resources.is_empty());
        assert!(removed
            .snapshot
            .manifest
            .tombstones
            .values()
            .any(|tombstone| matches!(tombstone.target, TombstoneTarget::Resource { .. })));
    }

    #[test]
    fn remote_bundle_listing_pages_more_than_one_thousand_bundles() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalBundleObjectStore::open(temp.path().join("store")).unwrap();
        for number in 1..=1_005_u64 {
            let id = bundle_id(number);
            let tag = BundleTag {
                schema_version: BUNDLE_SCHEMA_V4,
                bundle_id: id.clone(),
                display_name: format!("Bundle {}", number),
                kind: BundleKind::Project,
                generation: 1,
                updated_at: number,
                resources: 1,
                files: 1,
            };
            let key = repository_object_key(&id, TAG_FILE).unwrap();
            store
                .compare_and_swap(
                    &key,
                    &CasExpectation::Absent,
                    &serde_json::to_vec(&tag).unwrap(),
                )
                .unwrap();
        }
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let page = engine.list_remote_bundles(cursor.as_deref(), 113).unwrap();
            listed.extend(page.bundles.into_iter().map(|bundle| bundle.bundle_id));
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        assert_eq!(listed.len(), 1_005);
        assert!(listed.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn restore_plan_remaps_project_files_and_apply_creates_backup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        let target = temp.path().join("target-project");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(&source.join("CLAUDE.md"), b"portable guidance");
        write(&target.join("CLAUDE.md"), b"target edits");
        let request = CaptureRequest::for_project(&source);
        let inventory = discover_project(&request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let id = bundle_id(3);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Portable project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let write_action = plan
            .actions
            .iter()
            .find(|action| {
                action
                    .target_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("CLAUDE.md"))
            })
            .unwrap();
        assert!(write_action.requires_explicit_approval);
        let approved = BTreeSet::from([write_action.action_id.clone()]);
        let result = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(
            fs::read(target.join("CLAUDE.md")).unwrap(),
            b"portable guidance"
        );
        assert_eq!(result.backups.len(), 1);
        assert_eq!(
            fs::read(&result.backups[0].backup_path).unwrap(),
            b"target edits"
        );
        assert!(result
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
    }

    #[test]
    fn settings_and_mcp_apply_preserve_target_only_fields_and_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        let target = temp.path().join("target-project");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(
            &source.join(".claude/settings.json"),
            br#"{"model": "opus", "outputStyle": "concise", "permissions": {"allow": ["Bash(rm:*)"]}}"#,
        );
        write(
            &source.join(".mcp.json"),
            br#"{"mcpServers": {"tracker": {"command": "tracker-mcp", "env": {"TRACKER_TOKEN": "portable-secret"}}}}"#,
        );
        write(
            &target.join(".claude/settings.json"),
            br#"{"model": "haiku", "permissions": {"allow": ["Bash(ls:*)"]}}"#,
        );
        write(
            &target.join(".mcp.json"),
            br#"{"mcpServers": {"local-db": {"command": "db-mcp"}, "tracker": {"command": "old-tracker", "env": {"TRACKER_TOKEN": "target-secret"}}}}"#,
        );
        let request = CaptureRequest::for_project(&source);
        let inventory = discover_project(&request).unwrap();
        let resource_ids = [".claude/settings.json", ".mcp.json"].map(|name| {
            inventory
                .resources
                .iter()
                .find(|resource| resource.display_name == name)
                .unwrap()
                .resource_id
                .clone()
        });
        let recipe = recipe_for(resource_ids);
        let id = bundle_id(6);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Composed project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let settings_action = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::ApplySetting { .. }))
            .unwrap();
        let mcp_action = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::ReviewMcp { .. }))
            .unwrap();
        let approved = BTreeSet::from([
            settings_action.action_id.clone(),
            mcp_action.action_id.clone(),
        ]);
        let result = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(result
            .receipts
            .iter()
            .all(|receipt| receipt.status == ActionStatus::Applied));
        assert_eq!(result.backups.len(), 2, "both existing targets backed up");

        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(".claude/settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["model"], "opus", "portable field wins");
        assert_eq!(settings["outputStyle"], "concise", "portable field added");
        assert_eq!(
            settings["permissions"]["allow"][0], "Bash(ls:*)",
            "target-only permissions survive and never inherit portable ones"
        );

        let mcp: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(
            mcp["mcpServers"]["tracker"]["command"], "tracker-mcp",
            "portable non-secret field wins"
        );
        assert_eq!(
            mcp["mcpServers"]["tracker"]["env"]["TRACKER_TOKEN"], "target-secret",
            "target literal secret survives the portable ${{NAME}} placeholder"
        );
        assert_eq!(
            mcp["mcpServers"]["local-db"]["command"], "db-mcp",
            "target-only server survives"
        );
    }

    #[test]
    fn hook_and_toml_settings_apply_merge_by_identity_and_preserve_target_keys() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        let target = temp.path().join("target-project");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(
            &source.join(".codex/config.toml"),
            b"model = \"gpt-5\"\nweb_search = true\n",
        );
        write(
            &source.join(".codex/hooks.json"),
            br#"{"hooks": [{"name": "fmt", "command": "cargo fmt --check"}, {"name": "lint", "command": "cargo clippy"}]}"#,
        );
        write(
            &target.join(".codex/config.toml"),
            b"model = \"o3\"\nsandbox_mode = \"workspace-write\"\n",
        );
        write(
            &target.join(".codex/hooks.json"),
            br#"{"hooks": [{"name": "fmt", "command": "prettier --check ."}, {"name": "local-scan", "command": "scan.sh"}]}"#,
        );
        let request = CaptureRequest::for_project(&source);
        let inventory = discover_project(&request).unwrap();
        let resource_ids = [".codex/config.toml", ".codex/hooks.json"].map(|name| {
            inventory
                .resources
                .iter()
                .find(|resource| resource.display_name == name)
                .unwrap()
                .resource_id
                .clone()
        });
        let recipe = recipe_for(resource_ids);
        let id = bundle_id(7);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Codex project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let approved: BTreeSet<_> = plan
            .actions
            .iter()
            .filter(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::ApplySetting { .. } | RestoreActionKind::ReviewHook { .. }
                )
            })
            .map(|action| action.action_id.clone())
            .collect();
        assert_eq!(approved.len(), 2);
        let result = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(result
            .receipts
            .iter()
            .all(|receipt| receipt.status == ActionStatus::Applied));

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(target.join(".codex/config.toml")).unwrap())
                .unwrap();
        assert_eq!(config["model"].as_str(), Some("gpt-5"), "portable key wins");
        assert_eq!(
            config["web_search"].as_bool(),
            Some(true),
            "portable key added"
        );
        assert_eq!(
            config["sandbox_mode"].as_str(),
            Some("workspace-write"),
            "target-only TOML key survives"
        );

        let hooks: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(".codex/hooks.json")).unwrap()).unwrap();
        let list = hooks["hooks"].as_array().unwrap();
        let by_name = |name: &str| {
            list.iter()
                .find(|hook| hook["name"] == name)
                .unwrap_or_else(|| panic!("hook '{}' missing", name))
        };
        assert_eq!(
            by_name("fmt")["command"],
            "cargo fmt --check",
            "same-name hook merges by identity with portable winning"
        );
        assert_eq!(
            by_name("local-scan")["command"],
            "scan.sh",
            "target-only hook survives"
        );
        assert_eq!(
            by_name("lint")["command"],
            "cargo clippy",
            "portable-only hook is appended"
        );
        assert_eq!(list.len(), 3, "no duplicate hook entries");
    }

    #[test]
    fn claude_session_materializes_under_remapped_relative_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let source_child = source.join("apps/web");
        let target = temp.path().join("target");
        let target_child = target.join("apps/web");
        let source_claude = temp.path().join("source-claude");
        let target_claude = temp.path().join("target-claude");
        fs::create_dir_all(&source_child).unwrap();
        fs::create_dir_all(&target_child).unwrap();
        fs::create_dir_all(&source_claude).unwrap();
        fs::create_dir_all(&target_claude).unwrap();
        let transcript = format!(
            "{{\"type\":\"system\",\"sessionId\":\"session-a\",\"cwd\":{}}}\n",
            serde_json::to_string(source_child.to_str().unwrap()).unwrap()
        );
        write(
            &source_claude.join("projects/source-bucket/session-a.jsonl"),
            transcript.as_bytes(),
        );
        let request = CaptureRequest {
            project_root: source,
            codex_home: None,
            claude_home: Some(source_claude),
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "claude:session:session-a")
            .unwrap();
        assert_eq!(resource.relative_cwd.as_deref(), Some("apps/web"));
        let recipe = recipe_for([resource.resource_id.clone()]);
        let id = bundle_id(4);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Claude project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, Some(&target_claude));
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let action = plan
            .actions
            .iter()
            .find(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::MaterializeConversation { .. }
                )
            })
            .unwrap();
        let encoded_target = encode_claude_project_path(target_child.to_str().unwrap());
        assert!(action
            .target_path
            .as_deref()
            .unwrap()
            .contains(&encoded_target));
        let approved = BTreeSet::from([action.action_id.clone()]);
        engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(
            fs::read(action.target_path.as_ref().unwrap()).unwrap(),
            transcript.as_bytes()
        );
    }

    #[test]
    fn codex_session_materialization_rebinds_cwd_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let source_child = source.join("apps/web");
        let target = temp.path().join("target");
        let target_child = target.join("apps/web");
        let source_codex = temp.path().join("source-codex");
        let target_codex = temp.path().join("target-codex");
        fs::create_dir_all(&source_child).unwrap();
        fs::create_dir_all(&target_child).unwrap();
        fs::create_dir_all(&source_codex).unwrap();
        fs::create_dir_all(&target_codex).unwrap();

        let source_cwd = serde_json::to_string(source_child.to_str().unwrap()).unwrap();
        let historical_text = format!("keep historical path {}", source_child.display());
        let transcript = format!(
            concat!(
                "{{\"timestamp\":\"2026-07-18T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-a\",\"cwd\":{source_cwd}}}}}\r\n",
                "{{\"type\":\"response_item\",\"payload\":{{\"text\":{historical_text}}}}}\n",
                "{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":{source_cwd},\"workspace_roots\":[{source_cwd}]}}}}\n"
            ),
            source_cwd = source_cwd,
            historical_text = serde_json::to_string(&historical_text).unwrap(),
        );
        let session_relative = "sessions/2026/07/18/rollout-2026-07-18T00-00-00-thread-a.jsonl";
        write(&source_codex.join(session_relative), transcript.as_bytes());

        let request = CaptureRequest {
            project_root: source,
            codex_home: Some(source_codex),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "codex:session:thread-a")
            .unwrap();
        assert_eq!(resource.relative_cwd.as_deref(), Some("apps/web"));
        let resource_id = resource.resource_id.clone();
        let mut recipe = recipe_for([resource_id.clone()]);
        recipe
            .entries
            .get_mut(&ResourceId::parse(resource_id).unwrap())
            .unwrap()
            .apply_policy = ApplyPolicy::SafeFile;

        let id = bundle_id(8);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Codex project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, Some(&target_codex), None);

        // Simulate a session copied by an older release without path rebinding.
        let target_session = target_codex.join(session_relative);
        write(&target_session, transcript.as_bytes());
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        assert!(plan
            .actions
            .iter()
            .all(|action| !matches!(action.kind, RestoreActionKind::Manual { .. })));
        let action = plan
            .actions
            .iter()
            .find(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::MaterializeConversation {
                        provider: Provider::Codex,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(!action.requires_explicit_approval);

        let approved = BTreeSet::from([action.action_id.clone()]);
        let result = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(result.backups.len(), 1);

        let restored = fs::read(&target_session).unwrap();
        let rows = String::from_utf8(restored.clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let target_cwd = target_child.to_str().unwrap();
        assert_eq!(rows[0]["payload"]["cwd"], target_cwd);
        assert_eq!(rows[2]["payload"]["cwd"], target_cwd);
        assert_eq!(rows[1]["payload"]["text"], historical_text);
        assert_eq!(
            rows[2]["payload"]["workspace_roots"][0],
            source_child.to_str().unwrap(),
            "only structural cwd fields are rebound"
        );

        let repeat = engine
            .build_restore_plan(&fetched, &binding, 102, 0)
            .unwrap();
        let repeat_action = repeat
            .actions
            .iter()
            .find(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::MaterializeConversation {
                        provider: Provider::Codex,
                        ..
                    }
                )
            })
            .unwrap();
        let restored_sha = sha256(&restored);
        assert_eq!(
            repeat_action.expected_target_sha256.as_deref(),
            Some(restored_sha.as_str())
        );
        assert!(!repeat_action.requires_explicit_approval);
    }

    #[test]
    fn executable_skill_payload_requires_explicit_file_and_dependency_approval() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        let script = source.join(".agents/skills/release/run.sh");
        write(&script, b"#!/bin/sh\necho release\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let request = CaptureRequest::for_project(&source);
        let inventory = discover_project(&request).unwrap();
        let skill = inventory
            .resources
            .iter()
            .find(|resource| resource.kind == CaptureResourceKind::Skill)
            .unwrap();
        assert!(skill.dependency.is_some());
        let recipe = recipe_for([skill.resource_id.clone()]);
        let id = bundle_id(5);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Executable skill"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        assert_eq!(plan.actions.len(), 1);
        let install = fetched
            .dependency_actions
            .iter()
            .find(|action| matches!(action.kind, DependencyActionKind::InstallStandaloneSkill))
            .unwrap();
        assert!(install.requires_explicit_approval);
        let payload = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::WriteFile { .. }))
            .unwrap();
        assert!(payload.requires_explicit_approval);
        let approved = BTreeSet::from([payload.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(applied.deferred_dependencies.is_empty());
        assert_eq!(
            fs::read(target.join(".agents/skills/release/run.sh")).unwrap(),
            b"#!/bin/sh\necho release\n"
        );
    }

    #[test]
    fn plugin_install_intent_exists_only_in_the_dependency_plan_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        let mut request = CaptureRequest::for_project(&source);
        request.global_plugins.push(GlobalPluginSource {
            provider: CaptureProvider::Codex,
            plugin_id: "computer-use@openai-bundled".to_string(),
            marketplace: Some("openai-bundled".to_string()),
            source_type: None,
            source: None,
            observed_version: Some("1.0.0".to_string()),
            enabled: true,
            provided_skills: vec!["computer-use".to_string()],
        });
        let inventory = discover_project(&request).unwrap();
        let plugin = inventory
            .resources
            .iter()
            .find(|resource| resource.kind == CaptureResourceKind::Plugin)
            .unwrap();
        let recipe = recipe_for([plugin.resource_id.clone()]);
        let id = bundle_id(51);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Plugin intent"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        assert_eq!(fetched.dependency_actions.len(), 1);
        assert!(matches!(
            fetched.dependency_actions[0].kind,
            DependencyActionKind::InstallCodexPlugin
        ));
        let binding = active_binding(id, &target, None, None);
        let restore = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        assert!(restore.actions.is_empty());
    }

    /// Machine-A provider home with one global skill, ready for capture via
    /// the global inventory adapter.
    fn global_skill_fixture(temp: &Path) -> (PathBuf, CaptureRequest) {
        let project = temp.join("project");
        let codex_home = temp.join("codex-home");
        fs::create_dir_all(&project).unwrap();
        let skill = codex_home.join("skills/deploy");
        write(
            &skill.join("SKILL.md"),
            b"---\nname: deploy\n---\nDeploy helper\n",
        );
        write(&skill.join("bin/run.sh"), b"#!/bin/sh\necho deploy\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(skill.join("bin/run.sh"), fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let inventory = super::super::global_inventory::inventory_provider_home(
            CaptureProvider::Codex,
            &codex_home,
        );
        assert_eq!(inventory.standalone_skills.len(), 1);
        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex_home.clone()),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: inventory.standalone_skills,
            global_plugins: inventory.plugins,
            blocked_global_skills: inventory.blocked_skills,
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        (codex_home, request)
    }

    fn publish_skill_bundle(
        temp: &Path,
        request: &CaptureRequest,
        id: BundleId,
    ) -> BundleEngine<LocalBundleObjectStore> {
        let recipe = recipe_for(["codex:standalone-skill:deploy".to_string()]);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id, "Global skill"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        engine
    }

    #[test]
    fn custom_skill_installs_into_mapped_provider_home() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(21);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        // Machine B: fresh project and provider home.
        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let install = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .expect("one custom-skill install action");
        assert!(install.requires_explicit_approval);
        assert!(install.expected_target_sha256.is_none());
        assert!(!plan
            .actions
            .iter()
            .any(|action| matches!(action.kind, RestoreActionKind::WriteFile { .. })));

        // Keep is the default: an unapproved action mutates nothing.
        let skipped = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &BTreeSet::new(),
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(skipped
            .receipts
            .iter()
            .all(|receipt| receipt.status == ActionStatus::Skipped));
        assert!(!target_home.join("skills/deploy").exists());

        let approved = plan
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                102,
            )
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::InstallCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Applied);
        assert!(receipt.target_sha256_after.is_some());
        assert_eq!(
            fs::read(target_home.join("skills/deploy/bin/run.sh")).unwrap(),
            b"#!/bin/sh\necho deploy\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(target_home.join("skills/deploy/bin/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit survives install");
            assert_eq!(mode & 0o7000, 0, "set-id bits are stripped");
        }

        // Idempotent replan: a matching target is an install no-op pinned to
        // the same digest.
        let replan = engine
            .build_restore_plan(&fetched, &binding, 103, 0)
            .unwrap();
        let noop = replan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .unwrap();
        assert_eq!(noop.expected_target_sha256, noop.source_sha256);
    }

    #[test]
    fn custom_skill_overwrite_is_digest_pinned_and_backed_up() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(22);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        // A different local skill already occupies the target, including an
        // extra file the cloud version does not carry.
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nLocal variant\n",
        );
        write(&target_home.join("skills/deploy/local-note.txt"), b"mine");
        let binding = active_binding(id, &target_project, Some(&target_home), None);

        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let overwrite = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::OverwriteCustomSkill { .. }))
            .expect("a different target requires an explicit overwrite");
        assert!(overwrite.expected_target_sha256.is_some());
        assert_ne!(overwrite.expected_target_sha256, overwrite.source_sha256);

        let approved = BTreeSet::from([overwrite.action_id.clone()]);
        let backups_root = temp.path().join("backups");
        let applied = engine
            .apply_restore_plan(&fetched, &binding, &plan, &approved, &backups_root, 101)
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::OverwriteCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Applied);
        // Whole-directory replacement removes files deleted by the cloud
        // version instead of leaving stale code behind.
        assert!(!target_home.join("skills/deploy/local-note.txt").exists());
        assert_eq!(
            fs::read(target_home.join("skills/deploy/SKILL.md")).unwrap(),
            b"---\nname: deploy\n---\nDeploy helper\n"
        );
        // The displaced directory is recoverable from the plan backup.
        assert_eq!(applied.backups.len(), 1);
        let backup_dir = PathBuf::from(&applied.backups[0].backup_path);
        assert_eq!(
            fs::read(backup_dir.join("local-note.txt")).unwrap(),
            b"mine"
        );
    }

    #[test]
    fn custom_skill_target_change_after_planning_aborts_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(23);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nLocal variant\n",
        );
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let overwrite = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::OverwriteCustomSkill { .. }))
            .unwrap();

        // The target changes between review and apply.
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nEdited after review\n",
        );
        let approved = BTreeSet::from([overwrite.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::OverwriteCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Failed);
        assert!(receipt
            .error
            .as_deref()
            .unwrap()
            .contains("changed after planning"));
        assert_eq!(
            fs::read(target_home.join("skills/deploy/SKILL.md")).unwrap(),
            b"---\nname: deploy\n---\nEdited after review\n"
        );
    }

    #[test]
    fn custom_skill_preserves_install_directory_when_declared_name_differs() {
        let temp = tempfile::tempdir().unwrap();
        let source_project = temp.path().join("source-project");
        let source_home = temp.path().join("source-codex");
        fs::create_dir_all(&source_project).unwrap();
        write(
            &source_home.join("skills/capture-lsservice-detail/SKILL.md"),
            b"---\nname: get-real-hardware-rh-service\n---\nRun ~/.codex/skills/capture-lsservice-detail/scripts/run.py\n",
        );
        write(
            &source_home.join("skills/capture-lsservice-detail/scripts/run.py"),
            b"print('ok')\n",
        );
        let inventory = super::super::global_inventory::inventory_provider_home(
            CaptureProvider::Codex,
            &source_home,
        );
        assert!(inventory.blocked_skills.is_empty());
        assert_eq!(inventory.standalone_skills.len(), 1);
        let request = CaptureRequest {
            project_root: source_project,
            codex_home: Some(source_home),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: inventory.standalone_skills,
            global_plugins: inventory.plugins,
            blocked_global_skills: inventory.blocked_skills,
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let resource_id = "codex:standalone-skill:get-real-hardware-rh-service";
        let recipe = recipe_for([resource_id.to_string()]);
        let id = bundle_id(24);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish(PublishBundleRequest {
                identity: identity(id.clone(), "Differently named skill"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                expected_head: PublishExpectation::Absent,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let descriptor = fetched
            .snapshot
            .manifest
            .resources
            .get(&ResourceId::parse(resource_id).unwrap())
            .unwrap();
        assert_eq!(
            descriptor
                .metadata
                .get("effective_name")
                .map(String::as_str),
            Some("get-real-hardware-rh-service")
        );
        assert_eq!(
            descriptor
                .metadata
                .get("install_dir_name")
                .map(String::as_str),
            Some("capture-lsservice-detail")
        );

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let install = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .unwrap();
        match &install.kind {
            RestoreActionKind::InstallCustomSkill { skill_name, .. } => {
                assert_eq!(skill_name, "get-real-hardware-rh-service")
            }
            _ => unreachable!(),
        }
        assert!(install
            .target_path
            .as_deref()
            .unwrap()
            .ends_with("skills/capture-lsservice-detail"));

        let approved = BTreeSet::from([install.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(applied
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
        assert!(target_home
            .join("skills/capture-lsservice-detail/SKILL.md")
            .is_file());
        assert!(!target_home
            .join("skills/get-real-hardware-rh-service")
            .exists());
    }

    #[test]
    fn unselected_global_resources_never_enter_the_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        // Discovery sees the skill, but the recipe selects nothing global.
        let captured = capture_selected(&request, &BTreeSet::new()).unwrap();
        assert!(captured.resources.is_empty());
        assert!(captured.files.is_empty());
        assert!(captured.dependency_actions.is_empty());
    }
}

const MAX_SKILL_TREE_FILES: usize = 4_096;
const MAX_SKILL_TREE_BYTES: u64 = 512 * 1024 * 1024;

/// The effective identity of a custom-skill resource: bound provider plus
/// the runtime-visible skill name recorded at capture. The physical install
/// directory is tracked independently because valid skills may declare a
/// different runtime name and may refer to their installed path internally.
struct CustomSkillIdentity {
    provider: Provider,
    effective_name: String,
    install_dir_name: String,
}

fn custom_skill_identity(descriptor: &ResourceDescriptor) -> Result<CustomSkillIdentity, String> {
    let provider = descriptor
        .provider
        .ok_or_else(|| format!("custom skill '{}' lacks a provider", descriptor.resource_id))?;
    let effective_name = descriptor
        .metadata
        .get("effective_name")
        .cloned()
        .unwrap_or_else(|| descriptor.display_name.clone());
    let install_dir_name = descriptor
        .metadata
        .get("install_dir_name")
        .cloned()
        // Schema-3 bundles created under adapter v1 used one name for both
        // identity and installation, so preserve that interpretation.
        .unwrap_or_else(|| effective_name.clone());
    domain::validate_skill_name("custom skill effective name", &effective_name)?;
    domain::validate_skill_name("custom skill install directory", &install_dir_name)?;
    Ok(CustomSkillIdentity {
        provider,
        effective_name,
        install_dir_name,
    })
}

fn custom_skill_logical_root(provider: Provider, install_dir_name: &str) -> String {
    let provider = match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    };
    format!("state/{}/skills/{}", provider, install_dir_name)
}

/// Manifest files belonging to one custom skill, keyed by their path relative
/// to the skill directory.
fn custom_skill_manifest_files<'a>(
    manifest: &'a BundleManifest,
    resource_id: &ResourceId,
    logical_root: &str,
) -> Result<Vec<(String, &'a LogicalPath, &'a BundleFileEntry)>, String> {
    let prefix = format!("{}/", logical_root);
    let mut files = Vec::new();
    for (logical_path, entry) in &manifest.files {
        if &entry.resource_id != resource_id {
            continue;
        }
        let relative = logical_path.as_str().strip_prefix(&prefix).ok_or_else(|| {
            format!(
                "custom skill file '{}' is outside its skill root '{}'",
                logical_path, logical_root
            )
        })?;
        files.push((relative.to_string(), logical_path, entry));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn entry_is_executable(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o111 != 0)
}

/// Canonical tree digest over relative path, file type/executability, and
/// content hash. Computable both from a manifest and from a live directory so
/// "already installed" and "changed after review" are byte-grounded claims.
fn skill_tree_digest(entries: &[(String, bool, String)]) -> String {
    let mut material = String::new();
    for (relative, executable, digest) in entries {
        material.push_str(relative);
        material.push('\0');
        material.push(if *executable { 'x' } else { 'r' });
        material.push('\0');
        material.push_str(digest);
        material.push('\n');
    }
    sha256(material.as_bytes())
}

fn custom_skill_source_digest(files: &[(String, &LogicalPath, &BundleFileEntry)]) -> String {
    let entries = files
        .iter()
        .map(|(relative, _, entry)| {
            (
                relative.clone(),
                entry_is_executable(entry.mode),
                entry.sha256.clone(),
            )
        })
        .collect::<Vec<_>>();
    skill_tree_digest(&entries)
}

enum SkillTargetState {
    Missing,
    Present { digest: String },
    Blocked { reason: String },
}

/// No-follow scan of a live skill directory. Symlinks, special files, and
/// oversized trees block classification instead of being read through.
fn custom_skill_target_state(dir: &Path) -> Result<SkillTargetState, String> {
    inspect_existing_ancestors_no_symlink(dir)?;
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SkillTargetState::Missing)
        }
        Err(error) => return Err(format!("inspect '{}': {}", dir.display(), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(SkillTargetState::Blocked {
            reason: "target is not a regular directory".to_string(),
        });
    }
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(dir).follow_links(false).max_depth(16) {
        let entry = entry.map_err(|error| format!("walk '{}': {}", dir.display(), error))?;
        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Ok(SkillTargetState::Blocked {
                reason: format!("'{}' is not a regular file", entry.path().display()),
            });
        }
        let relative = entry
            .path()
            .strip_prefix(dir)
            .map_err(|_| "skill tree escaped its root".to_string())?
            .to_str()
            .ok_or_else(|| "skill tree contains a non-UTF-8 path".to_string())?
            .replace('\\', "/");
        let meta = entry
            .metadata()
            .map_err(|error| format!("inspect '{}': {}", entry.path().display(), error))?;
        total_bytes = total_bytes.saturating_add(meta.len());
        if entries.len() >= MAX_SKILL_TREE_FILES || total_bytes > MAX_SKILL_TREE_BYTES {
            return Ok(SkillTargetState::Blocked {
                reason: "target skill tree is too large to verify".to_string(),
            });
        }
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read '{}': {}", entry.path().display(), error))?;
        let executable = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        entries.push((relative, executable, sha256(&bytes)));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(SkillTargetState::Present {
        digest: skill_tree_digest(&entries),
    })
}

/// Resolve the provider-home target directory for one custom skill.
fn custom_skill_target_dir(
    binding: &ProjectBinding,
    provider: Provider,
    install_dir_name: &str,
) -> Result<PathBuf, String> {
    let home = match provider {
        Provider::Codex => canonical_bound_home(binding.codex_home.as_deref(), "Codex")?,
        Provider::Claude => canonical_bound_home(binding.claude_home.as_deref(), "Claude")?,
    };
    safe_lexical_join(&home, &format!("skills/{}", install_dir_name))
}

/// Build the single typed action for one custom-skill resource: Install for a
/// missing or matching target, Overwrite pinned to the current target digest
/// for a different one, Manual when the target cannot be classified.
fn custom_skill_action(
    bundle: &FetchedBundle,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<RestoreAction, String> {
    let identity = custom_skill_identity(descriptor)?;
    let logical_root = custom_skill_logical_root(identity.provider, &identity.install_dir_name);
    let files = custom_skill_manifest_files(
        &bundle.snapshot.manifest,
        &descriptor.resource_id,
        &logical_root,
    )?;
    if files.is_empty() {
        return manual_action(
            &descriptor.resource_id,
            &format!(
                "Custom skill '{}' has no payload in this generation",
                identity.effective_name
            ),
        );
    }
    let target =
        match custom_skill_target_dir(binding, identity.provider, &identity.install_dir_name) {
            Ok(target) => target,
            Err(error) => {
                return manual_action(
                    &descriptor.resource_id,
                    &format!(
                        "Custom skill '{}' needs a mapped provider home: {}",
                        identity.effective_name, error
                    ),
                )
            }
        };
    let source_digest = custom_skill_source_digest(&files);
    let state = custom_skill_target_state(&target)?;
    let (kind, expected) = match state {
        SkillTargetState::Missing => (
            RestoreActionKind::InstallCustomSkill {
                provider: identity.provider,
                skill_name: identity.effective_name.clone(),
            },
            None,
        ),
        SkillTargetState::Present { digest } => {
            let kind = if digest == source_digest {
                RestoreActionKind::InstallCustomSkill {
                    provider: identity.provider,
                    skill_name: identity.effective_name.clone(),
                }
            } else {
                RestoreActionKind::OverwriteCustomSkill {
                    provider: identity.provider,
                    skill_name: identity.effective_name.clone(),
                }
            };
            (kind, Some(digest))
        }
        SkillTargetState::Blocked { reason } => {
            return manual_action(
                &descriptor.resource_id,
                &format!(
                    "Custom skill target '{}' cannot be replaced automatically: {}",
                    target.display(),
                    reason
                ),
            )
        }
    };
    let action = RestoreAction {
        action_id: action_id_for(&descriptor.resource_id, &logical_root, kind.action_type())?,
        resource_id: descriptor.resource_id.clone(),
        kind,
        target_path: Some(path_text(&target)?),
        source_sha256: Some(source_digest),
        expected_target_sha256: expected,
        expected_target_mode: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}

fn manual_action(resource_id: &ResourceId, message: &str) -> Result<RestoreAction, String> {
    let suffix = &sha256(message.as_bytes())[..24];
    let action = RestoreAction {
        action_id: ActionId::parse(format!("manual-{}", suffix))?,
        resource_id: resource_id.clone(),
        kind: RestoreActionKind::Manual {
            message: message.to_string(),
        },
        target_path: None,
        source_sha256: None,
        expected_target_sha256: None,
        expected_target_mode: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}

fn conversation_keep_local_action(
    resource_id: &ResourceId,
    logical_path: &LogicalPath,
    target: &Path,
    source_sha256: &str,
    target_sha256: &str,
) -> Result<RestoreAction, String> {
    let message = format!(
        "Conversation '{}' differs at '{}'; this Pull will keep the local version",
        logical_path,
        target.display()
    );
    let suffix = &sha256(message.as_bytes())[..24];
    let action = RestoreAction {
        action_id: ActionId::parse(format!("manual-{}", suffix))?,
        resource_id: resource_id.clone(),
        kind: RestoreActionKind::Manual { message },
        target_path: Some(path_text(target)?),
        source_sha256: Some(source_sha256.to_string()),
        expected_target_sha256: Some(target_sha256.to_string()),
        expected_target_mode: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}

fn is_conversation_keep_local_action(action: &RestoreAction) -> bool {
    matches!(action.kind, RestoreActionKind::Manual { .. })
        && (action.resource_id.as_str().starts_with("codex:session:")
            || action.resource_id.as_str().starts_with("claude:session:"))
        && action.target_path.is_some()
        && action.source_sha256.is_some()
        && action.expected_target_sha256.is_some()
}

fn continuation_action(
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<RestoreAction, String> {
    let target_cwd = remapped_resource_cwd(descriptor, binding)?;
    let command = match descriptor.kind {
        ResourceKind::CodexConversation => format!(
            "codex resume {} -C {}",
            descriptor.display_name,
            target_cwd.display()
        ),
        ResourceKind::ClaudeConversation => format!(
            "cd {} && claude --resume {}",
            target_cwd.display(),
            descriptor.display_name
        ),
        _ => return Err("continuation action requires a conversation resource".to_string()),
    };
    manual_action(&descriptor.resource_id, &command)
}

fn remapped_resource_cwd(
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<PathBuf, String> {
    let relative_cwd = descriptor.relative_cwd.as_deref().unwrap_or(".");
    if relative_cwd == "." {
        Ok(PathBuf::from(&binding.project_root))
    } else {
        safe_lexical_join(Path::new(&binding.project_root), relative_cwd)
    }
}

fn materialized_file_bytes(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
    source: &[u8],
) -> Result<Vec<u8>, String> {
    let logical = logical_path.as_str();
    if descriptor.kind == ResourceKind::CodexConversation
        && (logical.starts_with("state/codex/sessions/")
            || logical.starts_with("state/codex/archived_sessions/"))
    {
        return remap_codex_session_cwd(source, descriptor, binding);
    }
    Ok(source.to_vec())
}

/// Codex scopes its default resume picker to the `cwd` stored in a session's
/// structural metadata. A portable conversation therefore needs those fields
/// rebound to this replica's project path when it is materialized. Only the
/// top-level provider records are changed; user messages and historical text
/// remain byte-for-byte untouched.
fn remap_codex_session_cwd(
    source: &[u8],
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<Vec<u8>, String> {
    if descriptor.relative_cwd.is_none() {
        return Err(format!(
            "Codex session '{}' has no project-relative cwd",
            descriptor.display_name
        ));
    }
    let target_cwd = path_text(&remapped_resource_cwd(descriptor, binding)?)?;
    let mut output = Vec::with_capacity(source.len());
    let mut found_session_meta = false;

    for line in source.split_inclusive(|byte| *byte == b'\n') {
        let (json, ending): (&[u8], &[u8]) = if let Some(json) = line.strip_suffix(b"\r\n") {
            (json, b"\r\n")
        } else if let Some(json) = line.strip_suffix(b"\n") {
            (json, b"\n")
        } else {
            (line, b"")
        };
        if json.is_empty() {
            output.extend_from_slice(line);
            continue;
        }
        if !contains_bytes(json, b"session_meta") && !contains_bytes(json, b"turn_context") {
            output.extend_from_slice(line);
            continue;
        }

        let mut value: serde_json::Value = match serde_json::from_slice(json) {
            Ok(value) => value,
            Err(_) => {
                output.extend_from_slice(line);
                continue;
            }
        };
        let record_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if !matches!(
            record_type.as_deref(),
            Some("session_meta" | "turn_context")
        ) {
            output.extend_from_slice(line);
            continue;
        }
        let Some(cwd) = value
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|payload| payload.get_mut("cwd"))
            .filter(|cwd| cwd.is_string())
        else {
            output.extend_from_slice(line);
            continue;
        };
        if record_type.as_deref() == Some("session_meta") {
            found_session_meta = true;
        }
        if cwd.as_str() == Some(target_cwd.as_str()) {
            output.extend_from_slice(line);
            continue;
        }
        *cwd = serde_json::Value::String(target_cwd.clone());
        output.extend_from_slice(
            &serde_json::to_vec(&value)
                .map_err(|error| format!("serialize remapped Codex session row: {}", error))?,
        );
        output.extend_from_slice(ending);
    }

    if !found_session_meta {
        return Err(format!(
            "Codex session '{}' has no remappable session_meta cwd",
            descriptor.display_name
        ));
    }
    Ok(output)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn restore_kind_for_file(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    entry: &BundleFileEntry,
) -> Result<RestoreActionKind, String> {
    if logical_path.as_str() == "state/codex/session_index.jsonl" {
        return Ok(RestoreActionKind::MergeFile {
            logical_path: logical_path.clone(),
        });
    }
    Ok(match descriptor.kind {
        ResourceKind::CodexConversation => RestoreActionKind::MaterializeConversation {
            provider: Provider::Codex,
            logical_path: logical_path.clone(),
        },
        ResourceKind::ClaudeConversation => RestoreActionKind::MaterializeConversation {
            provider: Provider::Claude,
            logical_path: logical_path.clone(),
        },
        ResourceKind::ProjectContentFile => RestoreActionKind::WriteProjectFile {
            logical_path: logical_path.clone(),
            mode: entry.mode.unwrap_or(0o600) & 0o777,
            source_mtime: entry.source_mtime,
        },
        ResourceKind::Hook => RestoreActionKind::ReviewHook {
            definition_sha256: entry.sha256.clone(),
        },
        ResourceKind::McpServer => RestoreActionKind::ReviewMcp {
            definition_sha256: entry.sha256.clone(),
        },
        ResourceKind::Setting => RestoreActionKind::ApplySetting {
            provider: descriptor
                .provider
                .ok_or_else(|| "setting resource lacks provider".to_string())?,
            semantic_key: logical_path.as_str().to_string(),
        },
        _ if descriptor.apply_policy == ApplyPolicy::Merge => RestoreActionKind::MergeFile {
            logical_path: logical_path.clone(),
        },
        _ => RestoreActionKind::WriteFile {
            logical_path: logical_path.clone(),
        },
    })
}

fn action_needs_explicit_approval(
    descriptor: &ResourceDescriptor,
    target_digest: Option<&str>,
    source_digest: &str,
) -> bool {
    // The Codex index is a derived, project-filtered list. Restore composes it
    // by stable session ID, preserves target-only rows, and creates a backup,
    // so it is safe to accompany the already-default conversation restores.
    if descriptor.resource_id.as_str() == "codex:session-index" {
        return false;
    }
    matches!(
        descriptor.apply_policy,
        ApplyPolicy::ExplicitInstall
            | ApplyPolicy::ExplicitReview
            | ApplyPolicy::ManualOnly
            | ApplyPolicy::Merge
    ) || target_digest.is_some_and(|target| target != source_digest)
        || matches!(
            descriptor.kind,
            ResourceKind::Hook
                | ResourceKind::McpServer
                | ResourceKind::Setting
                | ResourceKind::StandaloneSkill
                | ResourceKind::Plugin
        )
}

fn action_id_for(
    resource_id: &ResourceId,
    logical_path: &str,
    action_type: domain::RestoreActionType,
) -> Result<ActionId, String> {
    let material = format!("{}\0{}\0{:?}", resource_id, logical_path, action_type);
    ActionId::parse(format!("action-{}", &sha256(material.as_bytes())[..24]))
}

fn map_logical_target(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<Option<PathBuf>, String> {
    let logical = logical_path.as_str();
    if let Some(relative) = logical.strip_prefix("project/") {
        return Ok(Some(safe_lexical_join(
            Path::new(&binding.canonical_project_root),
            relative,
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/codex/skills/") {
        let home = canonical_bound_home(binding.codex_home.as_deref(), "Codex")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("skills/{}", relative),
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/claude/skills/") {
        let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("skills/{}", relative),
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/codex/sessions/") {
        let home = canonical_bound_home(binding.codex_home.as_deref(), "Codex")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("sessions/{}", relative),
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/codex/archived_sessions/") {
        let home = canonical_bound_home(binding.codex_home.as_deref(), "Codex")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("archived_sessions/{}", relative),
        )?));
    }
    if logical == "state/codex/session_index.jsonl" {
        return Ok(Some(
            canonical_bound_home(binding.codex_home.as_deref(), "Codex")?
                .join("session_index.jsonl"),
        ));
    }
    if let Some(relative) = logical.strip_prefix("state/claude/projects/") {
        let (_, file_relative) = relative
            .split_once('/')
            .ok_or_else(|| format!("invalid Claude project logical path '{}'", logical))?;
        let relative_cwd = descriptor.relative_cwd.as_deref().unwrap_or(".");
        let cwd = if relative_cwd == "." {
            PathBuf::from(&binding.project_root)
        } else {
            safe_lexical_join(Path::new(&binding.project_root), relative_cwd)?
        };
        let bucket = encode_claude_project_path(&path_text(&cwd)?);
        let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("projects/{}/{}", bucket, file_relative),
        )?));
    }
    for directory in ["file-history", "todos"] {
        if let Some(relative) = logical.strip_prefix(&format!("state/claude/{}/", directory)) {
            let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
            return Ok(Some(safe_lexical_join(
                &home,
                &format!("{}/{}", directory, relative),
            )?));
        }
    }
    if let Some(relative) = logical.strip_prefix("state/claude/memory/") {
        let (_, memory_relative) = relative
            .split_once('/')
            .ok_or_else(|| format!("invalid Claude memory path '{}'", logical))?;
        let relative_cwd = descriptor.relative_cwd.as_deref().unwrap_or(".");
        let cwd = if relative_cwd == "." {
            PathBuf::from(&binding.project_root)
        } else {
            safe_lexical_join(Path::new(&binding.project_root), relative_cwd)?
        };
        let bucket = encode_claude_project_path(&path_text(&cwd)?);
        let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("projects/{}/memory/{}", bucket, memory_relative),
        )?));
    }
    Ok(None)
}

fn project_content_target(
    logical_path: &LogicalPath,
    binding: &ProjectBinding,
) -> Result<PathBuf, String> {
    let relative = logical_path
        .as_str()
        .strip_prefix("project/")
        .ok_or_else(|| format!("'{}' is not project content", logical_path))?;
    safe_lexical_join(Path::new(&binding.canonical_project_root), relative)
}

fn directory_entry_digest(logical_path: &LogicalPath, mode: u32) -> String {
    sha256(format!("dir\0{}\0{:03o}", logical_path, mode & 0o777).as_bytes())
}

fn inspect_project_directory_target(path: &Path) -> Result<Option<u32>, String> {
    inspect_existing_ancestors_no_symlink(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "inspect project directory '{}': {}",
                path.display(),
                error
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "target '{}' is not a real no-follow directory",
            path.display()
        ));
    }
    Ok(Some(safe_mode(&metadata)))
}

fn project_action_logical_path(action: &RestoreAction) -> Option<LogicalPath> {
    match &action.kind {
        RestoreActionKind::WriteProjectFile { logical_path, .. }
        | RestoreActionKind::EnsureProjectDirectory { logical_path, .. }
        | RestoreActionKind::DeleteProjectFile { logical_path, .. }
        | RestoreActionKind::DeleteProjectDirectory { logical_path } => Some(logical_path.clone()),
        _ => None,
    }
}

fn project_action_depth(action: &RestoreAction) -> usize {
    project_action_logical_path(action)
        .map(|path| path.as_str().split('/').count())
        .unwrap_or(0)
}

fn logical_path_is_ancestor(directory: &LogicalPath, child: &LogicalPath) -> bool {
    child
        .as_str()
        .strip_prefix(directory.as_str())
        .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(unix)]
fn set_path_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))
        .map_err(|error| format!("set mode '{}': {}", path.display(), error))
}

#[cfg(not(unix))]
fn set_path_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn set_path_mtime_best_effort(path: &Path, source_mtime: u64) {
    if source_mtime == 0 {
        return;
    }
    if let Ok(file) = fs::OpenOptions::new().read(true).open(path) {
        let _ = file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(source_mtime));
    }
}

fn required_home<'a>(home: Option<&'a str>, provider: &str) -> Result<&'a Path, String> {
    home.map(Path::new)
        .ok_or_else(|| format!("{} home is required by this bundle", provider))
}

fn canonical_bound_home(home: Option<&str>, provider: &str) -> Result<PathBuf, String> {
    canonical_real_dir(
        required_home(home, provider)?,
        &format!("{} home", provider),
    )
}

fn encode_claude_project_path(path: &str) -> String {
    path.encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => byte as char,
            _ => '-',
        })
        .collect()
}

fn safe_lexical_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !root.is_absolute() || relative.is_empty() || relative.contains('\\') {
        return Err("unsafe restore path".to_string());
    }
    let mut result = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(value) => result.push(value),
            _ => return Err(format!("unsafe restore-relative path '{}'", relative)),
        }
    }
    Ok(result)
}

#[derive(Debug)]
struct RestoreTargetState {
    digest: Option<String>,
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
}

fn inspect_restore_target(path: &Path) -> Result<RestoreTargetState, String> {
    inspect_existing_ancestors_no_symlink(path)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RestoreTargetState {
                digest: None,
                bytes: None,
                mode: None,
            })
        }
        Err(e) => {
            return Err(format!(
                "inspect restore target '{}': {}",
                path.display(),
                e
            ))
        }
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!(
            "restore target '{}' is not a regular no-follow file",
            path.display()
        ));
    }
    if meta.len() > MAX_OBJECT_BYTES as u64 {
        return Err(format!("restore target '{}' is too large", path.display()));
    }
    let bytes =
        fs::read(path).map_err(|e| format!("read restore target '{}': {}", path.display(), e))?;
    Ok(RestoreTargetState {
        digest: Some(sha256(&bytes)),
        bytes: Some(bytes),
        mode: Some(safe_mode(&meta)),
    })
}

#[cfg(unix)]
fn safe_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn safe_mode(_metadata: &fs::Metadata) -> u32 {
    0o600
}

fn validate_target_for_logical(
    target: &Path,
    logical_path: &LogicalPath,
    binding: &ProjectBinding,
) -> Result<(), String> {
    let root = if logical_path.as_str().starts_with("project/") {
        PathBuf::from(&binding.canonical_project_root)
    } else if logical_path.as_str().starts_with("state/codex/") {
        canonical_bound_home(binding.codex_home.as_deref(), "Codex")?
    } else if logical_path.as_str().starts_with("state/claude/") {
        canonical_bound_home(binding.claude_home.as_deref(), "Claude")?
    } else {
        return Err(format!(
            "logical path '{}' has no writable target root",
            logical_path
        ));
    };
    let canonical_root = canonical_real_dir(&root, "restore root")?;
    if !target.starts_with(&root) {
        return Err(format!(
            "restore target '{}' escapes '{}';",
            target.display(),
            root.display()
        ));
    }
    let prospective = prospective_canonical_path(target)?;
    if !prospective.starts_with(&canonical_root) {
        return Err(format!(
            "restore target '{}' resolves outside '{}'",
            target.display(),
            root.display()
        ));
    }
    inspect_existing_ancestors_no_symlink(target)
}

fn inspect_existing_ancestors_no_symlink(path: &Path) -> Result<(), String> {
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(format!("symlink traversal at '{}'", existing.display()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("inspect '{}': {}", existing.display(), e)),
        }
        let Some(parent) = existing.parent() else {
            break;
        };
        if parent == existing {
            break;
        }
        existing = parent;
    }
    Ok(())
}

fn prospective_canonical_path(path: &Path) -> Result<PathBuf, String> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(format!("symlink traversal at '{}'", cursor.display()));
                }
                let mut resolved = fs::canonicalize(cursor)
                    .map_err(|e| format!("resolve '{}': {}", cursor.display(), e))?;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    format!("cannot resolve prospective path '{}'", path.display())
                })?;
                missing.push(name.to_os_string());
                cursor = cursor.parent().ok_or_else(|| {
                    format!("cannot resolve prospective path '{}'", path.display())
                })?;
            }
            Err(e) => return Err(format!("inspect '{}': {}", cursor.display(), e)),
        }
    }
}

pub(crate) fn write_target_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if bytes.len() > MAX_OBJECT_BYTES || mode & !0o777 != 0 {
        return Err("unsafe target bytes or mode".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("target '{}' has no parent", path.display()))?;
    let base = nearest_existing_directory(parent)?;
    create_safe_directory_tree(&base, parent)?;
    inspect_existing_ancestors_no_symlink(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create target temp in '{}': {}", parent.display(), e))?;
    temp.as_file_mut()
        .write_all(bytes)
        .map_err(|e| format!("write target '{}': {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file_mut()
            .set_permissions(fs::Permissions::from_mode(mode & 0o777))
            .map_err(|e| format!("set target mode '{}': {}", path.display(), e))?;
    }
    temp.as_file_mut()
        .sync_all()
        .map_err(|e| format!("sync target '{}': {}", path.display(), e))?;
    temp.persist(path)
        .map_err(|e| format!("publish target '{}': {}", path.display(), e.error))?;
    Ok(())
}

pub(crate) fn write_immutable_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Ok(existing) = fs::read(path) {
        return if existing == bytes {
            Ok(())
        } else {
            Err(format!(
                "backup '{}' already exists with different bytes",
                path.display()
            ))
        };
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("backup '{}' has no parent", path.display()))?;
    let base = nearest_existing_directory(parent)?;
    create_safe_directory_tree(&base, parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create backup temp: {}", e))?;
    temp.as_file_mut()
        .write_all(bytes)
        .map_err(|e| format!("write backup '{}': {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file_mut()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set backup mode: {}", e))?;
    }
    temp.persist_noclobber(path)
        .map_err(|e| format!("publish backup '{}': {}", path.display(), e.error))?;
    Ok(())
}

fn nearest_existing_directory(path: &Path) -> Result<PathBuf, String> {
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                return fs::canonicalize(cursor)
                    .map_err(|e| format!("resolve '{}': {}", cursor.display(), e))
            }
            Ok(_) => return Err(format!("'{}' is not a real directory", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                cursor = cursor
                    .parent()
                    .ok_or_else(|| format!("no existing ancestor for '{}'", path.display()))?;
            }
            Err(e) => return Err(format!("inspect '{}': {}", cursor.display(), e)),
        }
    }
}

fn create_safe_directory_tree(base: &Path, target: &Path) -> Result<(), String> {
    let base = fs::canonicalize(base)
        .map_err(|e| format!("resolve directory base '{}': {}", base.display(), e))?;
    let prospective = prospective_canonical_path(target)?;
    if !prospective.starts_with(&base) {
        return Err(format!(
            "directory target '{}' escapes '{}';",
            target.display(),
            base.display()
        ));
    }
    let relative = prospective
        .strip_prefix(&base)
        .map_err(|_| "directory containment changed".to_string())?;
    let mut cursor = base;
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("unsafe directory component".to_string());
        }
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(format!("'{}' is not a real directory", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&cursor)
                .map_err(|e| format!("create directory '{}': {}", cursor.display(), e))?,
            Err(e) => return Err(format!("inspect '{}': {}", cursor.display(), e)),
        }
    }
    Ok(())
}

fn validate_plan_pin(
    plan: &RestorePlan,
    bundle: &FetchedBundle,
    binding: &ProjectBinding,
    storage_id: &StorageId,
    now: u64,
) -> Result<(), String> {
    if now < plan.created_at || now > plan.expires_at {
        return Err("restore plan has expired or is not active yet".to_string());
    }
    if &plan.storage_id != storage_id
        || plan.storage_id != bundle.snapshot.storage_id
        || plan.bundle_id != bundle.snapshot.head.bundle_id
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.bundle_id != binding.bundle_id
        || plan.generation != bundle.snapshot.head.generation
        || plan.commit_id != bundle.snapshot.head.commit_id
        || plan.manifest_sha256 != bundle.snapshot.head.manifest_sha256
        || plan.binding_revision != binding.revision
    {
        return Err("restore plan pin no longer matches bundle/binding state".to_string());
    }
    Ok(())
}

fn receipt_for(
    action: &RestoreAction,
    status: ActionStatus,
    applied_at: u64,
    logical_path: Option<LogicalPath>,
    target_sha256_after: Option<String>,
    error: Option<String>,
) -> ApplyReceipt {
    ApplyReceipt {
        action_id: action.action_id.clone(),
        resource_id: action.resource_id.clone(),
        action_type: action.kind.action_type(),
        logical_path,
        source_sha256: action.source_sha256.clone(),
        target_path: action.target_path.clone(),
        target_sha256_after,
        status,
        applied_at,
        error,
        conversation: None,
    }
}

fn merge_jsonl_index(existing: Option<&[u8]>, incoming: &[u8]) -> Result<Vec<u8>, String> {
    let mut records = BTreeMap::<String, Vec<u8>>::new();
    for (origin, bytes) in [
        ("target", existing.unwrap_or_default()),
        ("bundle", incoming),
    ] {
        for raw in bytes.split(|byte| *byte == b'\n') {
            if raw.is_empty() {
                continue;
            }
            if raw.len() > 1024 * 1024 {
                return Err(format!("{} session-index row exceeds one MiB", origin));
            }
            let value: serde_json::Value = serde_json::from_slice(raw)
                .map_err(|e| format!("parse {} session-index row: {}", origin, e))?;
            let identity = json_identity(&value).unwrap_or_else(|| sha256(raw));
            if let Some(previous) = records.get(&identity) {
                if previous != raw && origin == "target" {
                    continue;
                }
            }
            // Bundle contribution wins for the same stable thread ID; the
            // target-only rows remain untouched.
            records.insert(identity, raw.to_vec());
        }
    }
    let mut output = Vec::new();
    for row in records.into_values() {
        output.extend_from_slice(&row);
        output.push(b'\n');
    }
    Ok(output)
}

fn json_identity(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in ["thread_id", "threadId", "session_id", "sessionId", "id"] {
                if let Some(value) = map.get(key).and_then(serde_json::Value::as_str) {
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
            map.values().find_map(json_identity)
        }
        serde_json::Value::Array(values) => values.iter().find_map(json_identity),
        _ => None,
    }
}

/// Compose the already-sanitized portable projection into the target's
/// project-scoped definition. Target-only fields (including local secrets,
/// permissions, trust, and unrelated MCP servers) survive. Literal target
/// env/header values also win over portable `${NAME}` requirements.
fn merge_portable_project_definition(
    existing: Option<&[u8]>,
    incoming: &[u8],
    logical_path: &str,
) -> Result<Vec<u8>, String> {
    let Some(existing) = existing else {
        return Ok(incoming.to_vec());
    };
    if logical_path.ends_with(".toml") {
        let target_text = std::str::from_utf8(existing)
            .map_err(|error| format!("target TOML is not UTF-8: {}", error))?;
        let incoming_text = std::str::from_utf8(incoming)
            .map_err(|error| format!("portable TOML is not UTF-8: {}", error))?;
        let mut target: toml::Value =
            toml::from_str(target_text).map_err(|error| format!("parse target TOML: {}", error))?;
        let incoming: toml::Value = toml::from_str(incoming_text)
            .map_err(|error| format!("parse portable TOML: {}", error))?;
        deep_merge_toml(&mut target, incoming);
        let mut output = toml::to_string_pretty(&target)
            .map_err(|error| format!("serialize composed TOML: {}", error))?
            .into_bytes();
        if !output.ends_with(b"\n") {
            output.push(b'\n');
        }
        return Ok(output);
    }
    if logical_path.ends_with(".json") {
        let mut target: serde_json::Value = serde_json::from_slice(existing)
            .map_err(|error| format!("parse target JSON: {}", error))?;
        let incoming: serde_json::Value = serde_json::from_slice(incoming)
            .map_err(|error| format!("parse portable JSON: {}", error))?;
        deep_merge_json(&mut target, incoming, false);
        let mut output = serde_json::to_vec_pretty(&target)
            .map_err(|error| format!("serialize composed JSON: {}", error))?;
        output.push(b'\n');
        return Ok(output);
    }
    Err(format!(
        "portable definition '{}' has no semantic composer",
        logical_path
    ))
}

fn deep_merge_toml(target: &mut toml::Value, incoming: toml::Value) {
    match (target, incoming) {
        (toml::Value::Table(target), toml::Value::Table(incoming)) => {
            for (key, value) in incoming {
                match target.get_mut(&key) {
                    Some(current) => deep_merge_toml(current, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, incoming) => *target = incoming,
    }
}

fn deep_merge_json(
    target: &mut serde_json::Value,
    incoming: serde_json::Value,
    preserve_existing_leaf: bool,
) {
    match (target, incoming) {
        (serde_json::Value::Object(target), serde_json::Value::Object(incoming)) => {
            for (key, value) in incoming {
                let preserve = preserve_existing_leaf
                    || key.eq_ignore_ascii_case("env")
                    || key.eq_ignore_ascii_case("headers");
                match target.get_mut(&key) {
                    Some(current) => deep_merge_json(current, value, preserve),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (serde_json::Value::Array(target), serde_json::Value::Array(incoming))
            if !preserve_existing_leaf =>
        {
            merge_json_array(target, incoming);
        }
        (target, incoming) if !preserve_existing_leaf => *target = incoming,
        _ => {}
    }
}

fn merge_json_array(target: &mut Vec<serde_json::Value>, incoming: Vec<serde_json::Value>) {
    for incoming_value in incoming {
        let identity = json_merge_identity(&incoming_value);
        if let Some((index, _)) = identity.as_ref().and_then(|identity| {
            target
                .iter()
                .enumerate()
                .find(|(_, value)| json_merge_identity(value).as_ref() == Some(identity))
        }) {
            deep_merge_json(&mut target[index], incoming_value, false);
        } else if !target.contains(&incoming_value) {
            target.push(incoming_value);
        }
    }
}

fn json_merge_identity(value: &serde_json::Value) -> Option<(String, String)> {
    let object = value.as_object()?;
    for key in ["name", "id", "pluginId", "plugin_id"] {
        if let Some(value) = object.get(key).and_then(serde_json::Value::as_str) {
            return Some((key.to_string(), value.to_string()));
        }
    }
    None
}

fn canonical_real_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} must be absolute: '{}'", label, path.display()));
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect {} '{}': {}", label, path.display(), e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} '{}' must be a real directory",
            label,
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| format!("resolve {} '{}': {}", label, path.display(), e))
}

fn canonical_or_prospective_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} must be absolute", label));
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(format!(
                "{} '{}' is not a real directory",
                label,
                path.display()
            ));
        }
    }
    prospective_canonical_path(path)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn path_text(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("path '{}' is not UTF-8", path.display()))?
        .to_string();
    domain::validate_absolute_clean_path("filesystem path", &text)?;
    Ok(text)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn random_named_id(prefix: &str) -> Result<String, String> {
    domain::generated_named_id(prefix)
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{:02x}", byte);
    }
    output
}

fn parse_bounded_json<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    label: &str,
) -> Result<T, String> {
    if bytes.len() > MAX_OBJECT_BYTES {
        return Err(format!("{} exceeds the JSON read limit", label));
    }
    serde_json::from_slice(bytes).map_err(|e| format!("parse {}: {}", label, e))
}

fn validate_store_path(value: &str, minimum_components: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(format!("unsafe store path '{}'", value.escape_debug()));
    }
    let components = value.split('/').collect::<Vec<_>>();
    if components.len() < minimum_components
        || components.iter().any(|component| {
            component.is_empty() || *component == "." || *component == ".." || component.len() > 255
        })
    {
        return Err(format!("unsafe store path '{}'", value.escape_debug()));
    }
    Ok(())
}

fn validate_bundle_relative_object_key(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 2048
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe bundle-relative object key '{}'", value));
    }
    if !(value == HEAD_FILE
        || value == TAG_FILE
        || value.starts_with("_manifests/")
        || value.starts_with("_commits/")
        || value.starts_with("_uploads/"))
    {
        return Err(format!("unknown bundle object namespace '{}'", value));
    }
    Ok(())
}

fn repository_object_key(bundle_id: &BundleId, relative: &str) -> Result<ObjectKey, String> {
    validate_bundle_relative_object_key(relative)?;
    ObjectKey::parse(format!("{}/{}/{}", REPOSITORY_PREFIX, bundle_id, relative))
}

fn checked_existing_object_path(root: &Path, key: &ObjectKey) -> Result<Option<PathBuf>, String> {
    let mut cursor = root.to_path_buf();
    for component in key.as_str().split('/') {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("symlink traversal in object key '{}'", key))
            }
            Ok(meta) if cursor != root.join(key.as_str()) && !meta.is_dir() => {
                return Err(format!("non-directory ancestor in object key '{}'", key))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("inspect object '{}': {}", key, e)),
        }
    }
    let canonical_parent = fs::canonicalize(
        cursor
            .parent()
            .ok_or_else(|| format!("object '{}' has no parent", key))?,
    )
    .map_err(|e| format!("resolve object parent '{}': {}", key, e))?;
    if !canonical_parent.starts_with(root) {
        return Err(format!("object '{}' escapes local store", key));
    }
    Ok(Some(cursor))
}

fn checked_create_object_path(root: &Path, key: &ObjectKey) -> Result<PathBuf, String> {
    let parts = key.as_str().split('/').collect::<Vec<_>>();
    let mut cursor = root.to_path_buf();
    for component in &parts[..parts.len() - 1] {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(format!("unsafe object ancestor '{}'", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&cursor)
                .map_err(|e| format!("create object directory '{}': {}", cursor.display(), e))?,
            Err(e) => {
                return Err(format!(
                    "inspect object directory '{}': {}",
                    cursor.display(),
                    e
                ))
            }
        }
    }
    let canonical_parent = fs::canonicalize(&cursor)
        .map_err(|e| format!("resolve object directory '{}': {}", cursor.display(), e))?;
    if !canonical_parent.starts_with(root) {
        return Err(format!("object '{}' escapes local store", key));
    }
    cursor.push(parts.last().unwrap());
    if let Ok(meta) = fs::symlink_metadata(&cursor) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("object target '{}' is unsafe", cursor.display()));
        }
    }
    Ok(cursor)
}

fn normalized_relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("'{}' is outside '{}';", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 store path '{}';", path.display()))?,
            ),
            _ => return Err(format!("unsafe store path '{}';", path.display())),
        }
    }
    Ok(parts.join("/"))
}

fn reconcile_manifest_content(
    recipe: &BundleRecipe,
    captured_descriptors: &BTreeMap<ResourceId, ResourceDescriptor>,
    captured: &CapturedResources,
    previous: Option<&BundleManifest>,
    updated_at: u64,
) -> Result<
    (
        BTreeMap<ResourceId, ResourceDescriptor>,
        BTreeMap<LogicalPath, BundleFileEntry>,
        BTreeMap<LogicalPath, BundleDirectoryEntry>,
        BTreeMap<String, Tombstone>,
    ),
    String,
> {
    for id in captured_descriptors.keys() {
        if !recipe.entries.contains_key(id) {
            return Err(format!("capture returned unselected resource '{}'", id));
        }
    }
    let captured_file_paths = captured
        .files
        .keys()
        .map(|path| LogicalPath::parse(path.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let captured_directory_paths = captured
        .directories
        .keys()
        .map(|path| LogicalPath::parse(path.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let captured_ids = captured_descriptors
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut resources = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    let mut tombstones = previous
        .map(|manifest| manifest.tombstones.clone())
        .unwrap_or_default();

    for id in recipe.entries.keys() {
        if let Some(descriptor) = captured_descriptors.get(id) {
            resources.insert(id.clone(), descriptor.clone());
        } else if let Some(descriptor) = previous.and_then(|manifest| manifest.resources.get(id)) {
            resources.insert(id.clone(), descriptor.clone());
            for (path, entry) in &previous.unwrap().files {
                if &entry.resource_id == id {
                    files.insert(path.clone(), entry.clone());
                }
            }
            for (path, entry) in &previous.unwrap().directories {
                if &entry.resource_id == id {
                    directories.insert(path.clone(), entry.clone());
                }
            }
        } else {
            return Err(format!(
                "selected resource '{}' is unavailable and has no previous captured version",
                id
            ));
        }
    }

    if let Some(previous) = previous {
        for (id, descriptor) in &previous.resources {
            if !recipe.entries.contains_key(id) {
                let last_sha256 = descriptor.metadata.get("content_sha256").cloned();
                let tombstone = Tombstone {
                    target: TombstoneTarget::Resource {
                        resource_id: id.clone(),
                    },
                    last_sha256,
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.canonical_key(), tombstone);
            }
        }
        for (path, old_entry) in &previous.files {
            let owner_removed = !recipe.entries.contains_key(&old_entry.resource_id);
            let recaptured_without_file = captured_ids.contains(&old_entry.resource_id)
                && !captured_file_paths.contains(path);
            if owner_removed || recaptured_without_file {
                let is_project_content = previous
                    .resources
                    .get(&old_entry.resource_id)
                    .is_some_and(|descriptor| descriptor.kind == ResourceKind::ProjectContentFile);
                let tombstone = Tombstone {
                    target: if is_project_content {
                        TombstoneTarget::ProjectContentFile {
                            resource_id: old_entry.resource_id.clone(),
                            logical_path: path.clone(),
                        }
                    } else {
                        TombstoneTarget::File {
                            resource_id: old_entry.resource_id.clone(),
                            logical_path: path.clone(),
                        }
                    },
                    last_sha256: Some(old_entry.sha256.clone()),
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.canonical_key(), tombstone);
                files.remove(path);
            }
        }
        for (path, old_entry) in &previous.directories {
            let owner_removed = !recipe.entries.contains_key(&old_entry.resource_id);
            let recaptured_without_directory = captured_ids.contains(&old_entry.resource_id)
                && !captured_directory_paths.contains(path);
            if owner_removed || recaptured_without_directory {
                let tombstone = Tombstone {
                    target: TombstoneTarget::ProjectContentDirectory {
                        resource_id: old_entry.resource_id.clone(),
                        logical_path: path.clone(),
                    },
                    last_sha256: None,
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.canonical_key(), tombstone);
                directories.remove(path);
            }
        }
    }

    tombstones.retain(|_, tombstone| match &tombstone.target {
        TombstoneTarget::Resource { resource_id } => !resources.contains_key(resource_id),
        TombstoneTarget::File {
            resource_id: _,
            logical_path,
        } => !files.contains_key(logical_path) && !captured_file_paths.contains(logical_path),
        TombstoneTarget::ProjectContentFile {
            resource_id: _,
            logical_path,
        } => !files.contains_key(logical_path) && !captured_file_paths.contains(logical_path),
        TombstoneTarget::ProjectContentDirectory {
            resource_id: _,
            logical_path,
        } => {
            !directories.contains_key(logical_path)
                && !captured_directory_paths.contains(logical_path)
        }
        TombstoneTarget::ConversationVariant { .. } | TombstoneTarget::ConversationGroup { .. } => {
            true
        }
    });
    Ok((resources, files, directories, tombstones))
}

fn dependencies_from_manifest(manifest: &BundleManifest) -> Result<Vec<DependencyAction>, String> {
    let mut actions = Vec::new();
    for descriptor in manifest.resources.values() {
        let Some(kind) = descriptor.metadata.get("dependency_kind") else {
            continue;
        };
        let kind = match kind.as_str() {
            "codex_plugin" => DependencyActionKind::InstallCodexPlugin,
            "claude_plugin" => DependencyActionKind::InstallClaudePlugin,
            "standalone_skill" => DependencyActionKind::InstallStandaloneSkill,
            other => return Err(format!("unknown dependency kind '{}'", other)),
        };
        let argv = descriptor
            .metadata
            .get("dependency_argv_json")
            .map(|json| serde_json::from_str::<Vec<String>>(json))
            .transpose()
            .map_err(|e| format!("parse dependency argv: {}", e))?
            .unwrap_or_default();
        if argv.len() > 64
            || argv.iter().any(|argument| {
                argument.len() > 4096 || argument.chars().any(|character| character == '\0')
            })
        {
            return Err(format!(
                "dependency '{}' has unsafe structured arguments",
                descriptor.resource_id
            ));
        }
        actions.push(DependencyAction {
            action_id: ActionId::parse(format!("dependency:{}", descriptor.resource_id))?,
            resource_id: descriptor.resource_id.clone(),
            kind,
            display_name: descriptor.display_name.clone(),
            provider: descriptor.provider,
            argv,
            requires_explicit_approval: true,
        });
    }
    actions.sort_by(|a, b| a.action_id.cmp(&b.action_id));
    Ok(actions)
}

fn validate_schema5_conversation_retention(
    previous: &BundleManifest,
    next: &BundleManifest,
) -> Result<(), String> {
    if previous.schema_version != BUNDLE_SCHEMA_V5 {
        return Ok(());
    }
    for (group_id, previous_group) in &previous.conversation_groups {
        let Some(next_group) = next.conversation_groups.get(group_id) else {
            let deleted = next.tombstones.values().any(|tombstone| {
                matches!(
                    &tombstone.target,
                    TombstoneTarget::ConversationGroup { conflict_group_id }
                        if conflict_group_id == group_id
                )
            });
            if !deleted {
                return Err(format!(
                    "schema-5 publication drops conversation group '{}' without a reviewed tombstone",
                    group_id
                ));
            }
            continue;
        };
        for variant_id in previous_group.variants.keys() {
            if next_group.variants.contains_key(variant_id) {
                continue;
            }
            let deleted = next.tombstones.values().any(|tombstone| {
                matches!(
                    &tombstone.target,
                    TombstoneTarget::ConversationVariant {
                        conflict_group_id,
                        variant_id: deleted_variant_id,
                    } if conflict_group_id == group_id && deleted_variant_id == variant_id
                )
            });
            if !deleted {
                return Err(format!(
                    "schema-5 publication drops conversation variant '{}'/ '{}' without a reviewed tombstone",
                    group_id, variant_id
                ));
            }
        }
    }
    Ok(())
}

fn schema5_manifest_is_semantic_noop(
    previous: &BundleManifest,
    candidate: &BundleManifest,
) -> bool {
    previous.bundle == candidate.bundle
        && previous.recipe == candidate.recipe
        && previous.captured_with.codec_versions == candidate.captured_with.codec_versions
        && previous.conversation_groups == candidate.conversation_groups
        && previous.resources == candidate.resources
        && schema5_files_are_semantically_equal(&previous.files, &candidate.files)
        && previous.directories == candidate.directories
        && previous.tombstones == candidate.tombstones
}

fn schema5_files_are_semantically_equal(
    previous: &BTreeMap<LogicalPath, BundleFileEntry>,
    candidate: &BTreeMap<LogicalPath, BundleFileEntry>,
) -> bool {
    previous.len() == candidate.len()
        && previous.iter().all(|(path, left)| {
            candidate.get(path).is_some_and(|right| {
                left.resource_id == right.resource_id
                    && left.sha256 == right.sha256
                    && left.size == right.size
                    && left.source_mtime == right.source_mtime
                    && left.mode == right.mode
            })
        })
}

fn file_delta(previous: Option<&BundleManifest>, current: &BundleManifest) -> (u64, u64, u64) {
    let Some(previous) = previous else {
        return (current.files.len() as u64, 0, 0);
    };
    let added = current
        .files
        .keys()
        .filter(|path| !previous.files.contains_key(*path))
        .count() as u64;
    let changed = current
        .files
        .iter()
        .filter(|(path, entry)| {
            previous
                .files
                .get(*path)
                .is_some_and(|old| old.sha256 != entry.sha256)
        })
        .count() as u64;
    let removed = previous
        .files
        .keys()
        .filter(|path| !current.files.contains_key(*path))
        .count() as u64;
    (added, changed, removed)
}

fn validate_bundle_snapshot_bytes(bundle: &FetchedBundle) -> Result<(), String> {
    bundle.snapshot.validate()?;
    if bundle.files.len() != bundle.snapshot.manifest.files.len() {
        return Err("fetched bundle file set differs from its manifest".to_string());
    }
    for (path, entry) in &bundle.snapshot.manifest.files {
        let bytes = bundle
            .files
            .get(path)
            .ok_or_else(|| format!("fetched bundle lacks '{}'", path))?;
        if bytes.len() as u64 != entry.size || sha256(bytes) != entry.sha256 {
            return Err(format!(
                "fetched file '{}' failed size/hash verification",
                path
            ));
        }
    }
    let bytes = serde_json::to_vec(&bundle.snapshot.manifest)
        .map_err(|e| format!("serialize fetched manifest: {}", e))?;
    if sha256(&bytes) != bundle.snapshot.head.manifest_sha256 {
        return Err("fetched manifest bytes differ from the pinned head hash".to_string());
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BackupRecord {
    pub action_id: ActionId,
    pub target_path: String,
    pub backup_path: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ApplyBundleResult {
    pub plan_id: PlanId,
    pub receipts: Vec<ApplyReceipt>,
    pub backups: Vec<BackupRecord>,
    pub deferred_dependencies: Vec<DependencyAction>,
}

impl<S: BundleObjectStore> BundleEngine<S> {
    /// Apply approved byte-materialization actions. New RestorePlans omit
    /// executable dependencies entirely; the legacy installer variants below
    /// remain only so a stale in-memory plan fails safely. This engine never
    /// shells out or silently activates executable configuration.
    pub fn apply_restore_plan(
        &self,
        bundle: &FetchedBundle,
        binding: &ProjectBinding,
        plan: &RestorePlan,
        approved_action_ids: &BTreeSet<ActionId>,
        backup_root: &Path,
        applied_at: u64,
    ) -> Result<ApplyBundleResult, String> {
        plan.validate()?;
        validate_bundle_snapshot_bytes(bundle)?;
        self.validate_binding(binding, Some(backup_root))?;
        validate_plan_pin(plan, bundle, binding, &self.storage_id, applied_at)?;
        let known_actions = plan
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = approved_action_ids
            .iter()
            .find(|action_id| !known_actions.contains(*action_id))
        {
            return Err(format!("approval references unknown action '{}'", unknown));
        }
        let mut effective_approved = approved_action_ids.clone();
        let approved_project_files = plan
            .actions
            .iter()
            .filter_map(|action| match &action.kind {
                RestoreActionKind::WriteProjectFile { logical_path, .. }
                    if effective_approved.contains(&action.action_id) =>
                {
                    Some(logical_path.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for action in &plan.actions {
            if let RestoreActionKind::EnsureProjectDirectory { logical_path, .. } = &action.kind {
                if approved_project_files
                    .iter()
                    .any(|file| logical_path_is_ancestor(logical_path, file))
                {
                    effective_approved.insert(action.action_id.clone());
                }
            }
        }
        let approves_project_content = plan.actions.iter().any(|action| {
            effective_approved.contains(&action.action_id)
                && matches!(
                    action.kind,
                    RestoreActionKind::WriteProjectFile { .. }
                        | RestoreActionKind::EnsureProjectDirectory { .. }
                        | RestoreActionKind::DeleteProjectFile { .. }
                        | RestoreActionKind::DeleteProjectDirectory { .. }
                )
        });
        if approves_project_content
            && plan
                .project_content_eligibility
                .as_ref()
                .is_some_and(|eligibility| {
                    eligibility.state != domain::ProjectFileSyncEligibilityState::Eligible
                })
        {
            return Err(
                "project content cannot be applied because this folder is Git-managed or eligibility is unknown"
                    .to_string(),
            );
        }

        let current = self
            .read_head(&plan.bundle_id)?
            .ok_or_else(|| "bundle head disappeared after planning".to_string())?;
        if current.0.generation != plan.generation
            || current.0.commit_id != plan.commit_id
            || current.0.manifest_sha256 != plan.manifest_sha256
        {
            return Err("bundle head changed after restore planning".to_string());
        }
        if !backup_root.is_absolute() {
            return Err("backup root must be absolute".to_string());
        }
        let backup_base = nearest_existing_directory(backup_root)?;
        create_safe_directory_tree(&backup_base, backup_root)?;
        let canonical_backup = canonical_real_dir(backup_root, "backup root")?;
        let plan_backup = canonical_backup.join(plan.plan_id.as_str());
        create_safe_directory_tree(&canonical_backup, &plan_backup)?;

        let mut receipts = Vec::new();
        let mut backups = Vec::new();
        let mut deferred = Vec::new();
        let dependencies_by_resource = bundle
            .dependency_actions
            .iter()
            .map(|action| (action.resource_id.clone(), action.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut prepared_directories = Vec::new();
        let mut ensure_actions = plan
            .actions
            .iter()
            .filter(|action| {
                effective_approved.contains(&action.action_id)
                    && matches!(
                        action.kind,
                        RestoreActionKind::EnsureProjectDirectory { .. }
                    )
            })
            .collect::<Vec<_>>();
        ensure_actions.sort_by_key(|action| project_action_depth(action));
        for action in ensure_actions {
            match self.prepare_project_directory(binding, action) {
                Ok(()) => prepared_directories.push(action),
                Err(error) => receipts.push(receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )),
            }
        }

        let mut approved_file_deletions = Vec::new();
        let mut approved_directory_deletions = Vec::new();

        for action in &plan.actions {
            if !effective_approved.contains(&action.action_id) {
                let keeps_local_conversation = is_conversation_keep_local_action(action);
                receipts.push(receipt_for(
                    action,
                    ActionStatus::Skipped,
                    applied_at,
                    None,
                    keeps_local_conversation
                        .then(|| action.expected_target_sha256.clone())
                        .flatten(),
                    Some(if keeps_local_conversation {
                        "local conversation was kept by the reviewed Pull".to_string()
                    } else {
                        "action was not approved".to_string()
                    }),
                ));
                continue;
            }
            match &action.kind {
                RestoreActionKind::EnsureProjectDirectory { .. } => {}
                RestoreActionKind::DeleteProjectFile { .. } => {
                    approved_file_deletions.push(action);
                }
                RestoreActionKind::DeleteProjectDirectory { .. } => {
                    approved_directory_deletions.push(action);
                }
                RestoreActionKind::InstallCustomSkill {
                    provider,
                    skill_name,
                }
                | RestoreActionKind::OverwriteCustomSkill {
                    provider,
                    skill_name,
                } => {
                    let receipt = self.apply_custom_skill_action(
                        bundle,
                        binding,
                        action,
                        *provider,
                        skill_name,
                        &plan_backup,
                        applied_at,
                        &mut backups,
                    );
                    receipts.push(match receipt {
                        Ok(receipt) => receipt,
                        Err(error) => receipt_for(
                            action,
                            ActionStatus::Failed,
                            applied_at,
                            None,
                            None,
                            Some(error),
                        ),
                    });
                }
                RestoreActionKind::InstallPlugin { .. }
                | RestoreActionKind::InstallStandaloneSkill { .. } => {
                    if let Some(dependency) = dependencies_by_resource.get(&action.resource_id) {
                        deferred.push(dependency.clone());
                    }
                    receipts.push(receipt_for(
                        action,
                        ActionStatus::Blocked,
                        applied_at,
                        None,
                        None,
                        Some(
                            "Installer action belongs to Global tools; refresh Pull review"
                                .to_string(),
                        ),
                    ));
                }
                RestoreActionKind::Manual { .. } => {
                    receipts.push(receipt_for(
                        action,
                        ActionStatus::Blocked,
                        applied_at,
                        None,
                        None,
                        Some(
                            "definition requires its provider-specific review/composer".to_string(),
                        ),
                    ));
                }
                RestoreActionKind::ReviewHook { .. }
                | RestoreActionKind::ReviewMcp { .. }
                | RestoreActionKind::ApplySetting { .. } => {
                    let receipt = self
                        .logical_path_for_review_action(bundle, action)
                        .and_then(|logical_path| {
                            self.apply_file_action(
                                bundle,
                                binding,
                                action,
                                &logical_path,
                                &plan_backup,
                                applied_at,
                                &mut backups,
                            )
                        });
                    receipts.push(match receipt {
                        Ok(receipt) => receipt,
                        Err(error) => receipt_for(
                            action,
                            ActionStatus::Failed,
                            applied_at,
                            None,
                            None,
                            Some(error),
                        ),
                    });
                }
                RestoreActionKind::WriteFile { logical_path }
                | RestoreActionKind::WriteProjectFile { logical_path, .. }
                | RestoreActionKind::MergeFile { logical_path }
                | RestoreActionKind::MaterializeConversation { logical_path, .. } => {
                    let receipt = self.apply_file_action(
                        bundle,
                        binding,
                        action,
                        logical_path,
                        &plan_backup,
                        applied_at,
                        &mut backups,
                    );
                    receipts.push(match receipt {
                        Ok(receipt) => receipt,
                        Err(error) => receipt_for(
                            action,
                            ActionStatus::Failed,
                            applied_at,
                            None,
                            None,
                            Some(error),
                        ),
                    });
                }
            }
        }

        approved_file_deletions
            .sort_by_key(|action| std::cmp::Reverse(project_action_depth(action)));
        for action in approved_file_deletions {
            let receipt = self.apply_project_file_deletion(
                binding,
                action,
                &plan_backup,
                applied_at,
                &mut backups,
            );
            receipts.push(receipt.unwrap_or_else(|error| {
                receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )
            }));
        }

        approved_directory_deletions
            .sort_by_key(|action| std::cmp::Reverse(project_action_depth(action)));
        for action in approved_directory_deletions {
            let receipt = self.apply_project_directory_deletion(binding, action, applied_at);
            receipts.push(receipt.unwrap_or_else(|error| {
                receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )
            }));
        }

        prepared_directories.sort_by_key(|action| std::cmp::Reverse(project_action_depth(action)));
        for action in prepared_directories {
            let receipt = self.finalize_project_directory(binding, action, applied_at);
            receipts.push(receipt.unwrap_or_else(|error| {
                receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )
            }));
        }
        deferred.sort_by(|a, b| a.action_id.cmp(&b.action_id));
        deferred.dedup_by(|a, b| a.action_id == b.action_id);
        Ok(ApplyBundleResult {
            plan_id: plan.plan_id.clone(),
            receipts,
            backups,
            deferred_dependencies: deferred,
        })
    }

    fn prepare_project_directory(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
    ) -> Result<(), String> {
        let RestoreActionKind::EnsureProjectDirectory { logical_path, .. } = &action.kind else {
            return Err("directory preparation received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project directory target differs from the pinned plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        match (
            inspect_project_directory_target(&target)?,
            action.expected_target_mode,
        ) {
            (Some(actual), Some(expected)) if actual == expected => Ok(()),
            (Some(_), Some(_)) => Err("project directory mode changed after planning".to_string()),
            (Some(_), None) => Err("project directory appeared after planning".to_string()),
            (None, Some(_)) => Err("project directory disappeared after planning".to_string()),
            (None, None) => {
                let parent = target
                    .parent()
                    .ok_or_else(|| "project directory has no parent".to_string())?;
                let parent_meta = fs::symlink_metadata(parent).map_err(|error| {
                    format!(
                        "inspect project directory parent '{}': {}",
                        parent.display(),
                        error
                    )
                })?;
                if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
                    return Err(format!(
                        "project directory parent '{}' is not a real directory",
                        parent.display()
                    ));
                }
                fs::create_dir(&target).map_err(|error| {
                    format!("create project directory '{}': {}", target.display(), error)
                })?;
                set_path_mode(&target, 0o700)
            }
        }
    }

    fn finalize_project_directory(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
        applied_at: u64,
    ) -> Result<ApplyReceipt, String> {
        let RestoreActionKind::EnsureProjectDirectory {
            logical_path,
            mode,
            source_mtime,
        } = &action.kind
        else {
            return Err("directory finalization received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        validate_target_for_logical(&target, logical_path, binding)?;
        if inspect_project_directory_target(&target)?.is_none() {
            return Err("project directory disappeared before finalization".to_string());
        }
        set_path_mode(&target, *mode)?;
        set_path_mtime_best_effort(&target, *source_mtime);
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            action.source_sha256.clone(),
            None,
        ))
    }

    fn apply_project_file_deletion(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        let RestoreActionKind::DeleteProjectFile {
            logical_path,
            last_sha256,
        } = &action.kind
        else {
            return Err("file deletion received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project-file deletion target differs from the plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        let before = inspect_restore_target(&target)?;
        let Some(bytes) = before.bytes else {
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                None,
                None,
            ));
        };
        if before.digest.as_deref() != Some(last_sha256.as_str())
            || action.expected_target_sha256.as_deref() != Some(last_sha256.as_str())
        {
            return Err("local project file changed; deletion was blocked".to_string());
        }
        if action
            .expected_target_mode
            .zip(before.mode)
            .is_some_and(|(expected, actual)| expected != actual)
        {
            return Err("local project file mode changed; deletion was blocked".to_string());
        }
        let backup_path = plan_backup.join(format!("{}.delete.bak", action.action_id.as_str()));
        write_immutable_backup(&backup_path, &bytes)?;
        if sha256(
            &fs::read(&backup_path)
                .map_err(|error| format!("verify deletion backup: {}", error))?,
        ) != *last_sha256
        {
            return Err("project-file deletion backup failed verification".to_string());
        }
        backups.push(BackupRecord {
            action_id: action.action_id.clone(),
            target_path: path_text(&target)?,
            backup_path: path_text(&backup_path)?,
            sha256: last_sha256.clone(),
        });
        fs::remove_file(&target)
            .map_err(|error| format!("delete project file '{}': {}", target.display(), error))?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            None,
            None,
        ))
    }

    fn apply_project_directory_deletion(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
        applied_at: u64,
    ) -> Result<ApplyReceipt, String> {
        let RestoreActionKind::DeleteProjectDirectory { logical_path } = &action.kind else {
            return Err("directory deletion received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project-directory deletion target differs from the plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        let Some(actual_mode) = inspect_project_directory_target(&target)? else {
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                None,
                None,
            ));
        };
        if action
            .expected_target_mode
            .is_some_and(|expected| expected != actual_mode)
        {
            return Err("local project directory mode changed; deletion was blocked".to_string());
        }
        if fs::read_dir(&target)
            .map_err(|error| format!("read project directory '{}': {}", target.display(), error))?
            .next()
            .is_some()
        {
            return Err(
                "project directory is not empty; recursive deletion is forbidden".to_string(),
            );
        }
        fs::remove_dir(&target).map_err(|error| {
            format!(
                "delete empty project directory '{}': {}",
                target.display(),
                error
            )
        })?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            None,
            None,
        ))
    }

    fn logical_path_for_review_action(
        &self,
        bundle: &FetchedBundle,
        action: &RestoreAction,
    ) -> Result<LogicalPath, String> {
        let mut matches = bundle
            .snapshot
            .manifest
            .files
            .iter()
            .filter(|(_, entry)| entry.resource_id == action.resource_id)
            .map(|(logical_path, _)| logical_path.clone());
        let logical_path = matches
            .next()
            .ok_or_else(|| "review action has no portable definition bytes".to_string())?;
        if matches.next().is_some() {
            return Err(
                "review action spans multiple files and needs a provider-specific composer"
                    .to_string(),
            );
        }
        Ok(logical_path)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_file_action(
        &self,
        bundle: &FetchedBundle,
        binding: &ProjectBinding,
        action: &RestoreAction,
        logical_path: &LogicalPath,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        let entry = bundle
            .snapshot
            .manifest
            .files
            .get(logical_path)
            .ok_or_else(|| format!("plan references missing file '{}'", logical_path))?;
        if entry.resource_id != action.resource_id {
            return Err("restore action resource changed after planning".to_string());
        }
        let descriptor = bundle
            .snapshot
            .manifest
            .resources
            .get(&entry.resource_id)
            .ok_or_else(|| "restore action resource is missing".to_string())?;
        let bytes = bundle
            .files
            .get(logical_path)
            .ok_or_else(|| format!("fetched bytes for '{}' are missing", logical_path))?;
        if bytes.len() as u64 != entry.size || sha256(bytes) != entry.sha256 {
            return Err(format!(
                "fetched bytes for '{}' failed verification",
                logical_path
            ));
        }
        let materialized_bytes = materialized_file_bytes(logical_path, descriptor, binding, bytes)?;
        let materialized_sha256 = sha256(&materialized_bytes);
        let target = map_logical_target(logical_path, descriptor, binding)?
            .ok_or_else(|| format!("'{}' has no file materializer", logical_path))?;
        let planned_target = action
            .target_path
            .as_deref()
            .ok_or_else(|| "file restore action has no target".to_string())?;
        if Path::new(planned_target) != target {
            return Err("restore target differs from the pinned plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        let before = inspect_restore_target(&target)?;
        match (&action.expected_target_sha256, &before.digest) {
            (None, None) => {}
            (Some(expected), Some(actual)) if expected == actual => {}
            _ => return Err("restore target changed after planning".to_string()),
        }
        if action.expected_target_mode != before.mode
            && matches!(action.kind, RestoreActionKind::WriteProjectFile { .. })
        {
            return Err("project-file mode changed after planning".to_string());
        }
        if before.digest.as_deref() == Some(materialized_sha256.as_str()) {
            if let RestoreActionKind::WriteProjectFile {
                mode, source_mtime, ..
            } = &action.kind
            {
                if (entry.mode.unwrap_or(0o600) & 0o777) != (*mode & 0o777) {
                    return Err("project-file metadata changed after planning".to_string());
                }
                set_path_mode(&target, *mode)?;
                set_path_mtime_best_effort(&target, *source_mtime);
            }
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                Some(materialized_sha256),
                None,
            ));
        }

        if let Some(existing) = before.bytes.as_ref() {
            let backup_path = plan_backup.join(format!("{}.bak", action.action_id.as_str()));
            write_immutable_backup(&backup_path, existing)?;
            backups.push(BackupRecord {
                action_id: action.action_id.clone(),
                target_path: path_text(&target)?,
                backup_path: path_text(&backup_path)?,
                sha256: sha256(existing),
            });
        }
        let write_bytes = if logical_path.as_str() == "state/codex/session_index.jsonl" {
            merge_jsonl_index(before.bytes.as_deref(), bytes)?
        } else if matches!(
            descriptor.kind,
            ResourceKind::Setting | ResourceKind::McpServer | ResourceKind::Hook
        ) {
            merge_portable_project_definition(
                before.bytes.as_deref(),
                bytes,
                logical_path.as_str(),
            )?
        } else {
            materialized_bytes
        };
        write_target_atomic(&target, &write_bytes, entry.mode.unwrap_or(0o600))?;
        if let RestoreActionKind::WriteProjectFile {
            mode, source_mtime, ..
        } = &action.kind
        {
            if (entry.mode.unwrap_or(0o600) & 0o777) != (*mode & 0o777) {
                return Err("project-file metadata changed after planning".to_string());
            }
            set_path_mode(&target, *mode)?;
            set_path_mtime_best_effort(&target, *source_mtime);
        }
        let after = sha256(&write_bytes);
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            Some(after),
            None,
        ))
    }

    /// Materialize one custom skill as a whole-directory transaction: stage
    /// the complete verified tree next to the target, back up and journal the
    /// existing directory, swap by rename, and roll back on failure. Runs
    /// under the canonical provider-home operation lock.
    #[allow(clippy::too_many_arguments)]
    fn apply_custom_skill_action(
        &self,
        bundle: &FetchedBundle,
        binding: &ProjectBinding,
        action: &RestoreAction,
        provider: Provider,
        skill_name: &str,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        domain::validate_skill_name("custom skill name", skill_name)?;
        let descriptor = bundle
            .snapshot
            .manifest
            .resources
            .get(&action.resource_id)
            .ok_or_else(|| "custom-skill resource is missing from the manifest".to_string())?;
        if descriptor.kind != ResourceKind::StandaloneSkill {
            return Err("action resource is not a custom skill".to_string());
        }
        let identity = custom_skill_identity(descriptor)?;
        if identity.provider != provider || identity.effective_name != skill_name {
            return Err("custom-skill action identity differs from its descriptor".to_string());
        }
        let logical_root = custom_skill_logical_root(identity.provider, &identity.install_dir_name);
        let files = custom_skill_manifest_files(
            &bundle.snapshot.manifest,
            &action.resource_id,
            &logical_root,
        )?;
        if files.is_empty() {
            return Err("custom skill has no payload files".to_string());
        }
        let source_digest = custom_skill_source_digest(&files);
        if action.source_sha256.as_deref() != Some(source_digest.as_str()) {
            return Err("custom-skill payload changed after planning".to_string());
        }
        let target =
            custom_skill_target_dir(binding, identity.provider, &identity.install_dir_name)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("custom-skill target differs from the pinned plan".to_string());
        }
        let skills_dir = target
            .parent()
            .ok_or_else(|| "custom-skill target has no parent".to_string())?
            .to_path_buf();
        let home = skills_dir
            .parent()
            .ok_or_else(|| "skills directory has no parent".to_string())?;
        inspect_existing_ancestors_no_symlink(&skills_dir)?;
        if !skills_dir.exists() {
            fs::create_dir_all(&skills_dir)
                .map_err(|error| format!("create '{}': {}", skills_dir.display(), error))?;
        }
        let canonical_skills = canonical_real_dir(&skills_dir, "global skills directory")?;
        let canonical_home = canonical_real_dir(home, "provider home")?;
        if !canonical_skills.starts_with(&canonical_home) {
            return Err("global skills directory escapes the provider home".to_string());
        }

        // Canonically equivalent provider homes resolve to the same lock file,
        // so concurrent operations on one home serialize regardless of path
        // spelling. `.`-prefixed names stay invisible to inventory and sync.
        let lock_path = canonical_skills.join(".agent-sync.lock");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("open provider-home lock: {}", error))?;
        lock_file
            .lock()
            .map_err(|error| format!("acquire provider-home lock: {}", error))?;

        // Revalidate the live target under the lock; approval is pinned to
        // the reviewed digest and a changed target aborts without mutation.
        let state = custom_skill_target_state(&target)?;
        let current_digest = match (&state, &action.expected_target_sha256) {
            (SkillTargetState::Missing, None) => None,
            (SkillTargetState::Present { digest }, _) if digest == &source_digest => {
                // Already-matching content: record the installed claim
                // without rewriting anything (install no-op / adopt).
                return Ok(receipt_for(
                    action,
                    ActionStatus::Applied,
                    applied_at,
                    None,
                    Some(source_digest),
                    None,
                ));
            }
            (SkillTargetState::Present { digest }, Some(expected)) if digest == expected => {
                Some(digest.clone())
            }
            (SkillTargetState::Blocked { reason }, _) => {
                return Err(format!("custom-skill target is blocked: {}", reason))
            }
            _ => return Err("custom-skill target changed after planning".to_string()),
        };

        // Stage the complete tree on the same filesystem as the target so the
        // final activation is a rename, then verify every staged byte.
        let staging = canonical_skills.join(format!(".agent-sync-staging-{}", action.action_id));
        if staging.exists() {
            fs::remove_dir_all(&staging)
                .map_err(|error| format!("clear stale staging: {}", error))?;
        }
        let staging_result = (|| -> Result<(), String> {
            for (relative, logical_path, entry) in &files {
                let bytes = bundle
                    .files
                    .get(*logical_path)
                    .ok_or_else(|| format!("fetched bytes for '{}' are missing", logical_path))?;
                if bytes.len() as u64 != entry.size || sha256(bytes) != entry.sha256 {
                    return Err(format!("fetched '{}' failed verification", logical_path));
                }
                let staged = safe_lexical_join(&staging, relative)?;
                if let Some(parent) = staged.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
                }
                // Set-id and non-permission bits were stripped at capture;
                // strip again on the write path for defense in depth.
                write_target_atomic(&staged, bytes, entry.mode.unwrap_or(0o600) & 0o777)?;
            }
            match custom_skill_target_state(&staging)? {
                SkillTargetState::Present { digest } if digest == source_digest => Ok(()),
                _ => Err("staged custom skill failed tree verification".to_string()),
            }
        })();
        if let Err(error) = staging_result {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        // Journal before mutating the target so an interrupted swap is
        // detectable and recoverable from the retained plan backup.
        let backup_dir = plan_backup.join(format!("{}.skill-backup", action.action_id));
        let journal_path = plan_backup.join(format!("{}.journal.json", action.action_id));
        let write_journal = |phase: &str| -> Result<(), String> {
            let journal = serde_json::json!({
                "action_id": action.action_id.as_str(),
                "resource_id": action.resource_id.as_str(),
                "target": path_text(&target)?,
                "staging": path_text(&staging)?,
                "backup": path_text(&backup_dir)?,
                "source_tree_sha256": source_digest,
                "phase": phase,
            });
            fs::write(&journal_path, journal.to_string())
                .map_err(|error| format!("write custom-skill journal: {}", error))
        };
        write_journal("staged")?;

        let displaced = canonical_skills.join(format!(".agent-sync-old-{}", action.action_id));
        if let Some(current_digest) = &current_digest {
            if let Err(error) = copy_skill_tree(&target, &backup_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("custom-skill backup failed: {}", error));
            }
            backups.push(BackupRecord {
                action_id: action.action_id.clone(),
                target_path: path_text(&target)?,
                backup_path: path_text(&backup_dir)?,
                sha256: current_digest.clone(),
            });
            write_journal("backed-up")?;
            if let Err(error) = fs::rename(&target, &displaced) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("displace existing skill: {}", error));
            }
        }
        if let Err(error) = fs::rename(&staging, &target) {
            // Restore the displaced directory before reporting failure; the
            // journal and backup stay behind for manual recovery if this
            // rollback rename also fails.
            if current_digest.is_some() {
                let _ = fs::rename(&displaced, &target);
            }
            let _ = fs::remove_dir_all(&staging);
            write_journal("rolled-back")?;
            return Err(format!("activate staged skill: {}", error));
        }
        if current_digest.is_some() {
            let _ = fs::remove_dir_all(&displaced);
        }
        let verified = match custom_skill_target_state(&target)? {
            SkillTargetState::Present { digest } if digest == source_digest => digest,
            _ => {
                write_journal("verify-failed")?;
                return Err(
                    "installed custom skill failed post-activation verification".to_string()
                );
            }
        };
        write_journal("complete")?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            None,
            Some(verified),
            None,
        ))
    }
}

/// Bounded no-follow recursive copy used for pre-overwrite backups. The
/// source tree was already verified to contain only regular files.
fn copy_skill_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("create '{}': {}", destination.display(), error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(destination, fs::Permissions::from_mode(0o700));
    }
    for entry in WalkDir::new(source).follow_links(false).max_depth(16) {
        let entry = entry.map_err(|error| format!("walk '{}': {}", source.display(), error))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|_| "backup tree escaped its root".to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        let file_type = entry.file_type();
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("create '{}': {}", target.display(), error))?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|error| format!("copy '{}': {}", entry.path().display(), error))?;
        } else {
            return Err(format!(
                "backup source '{}' is not a regular file",
                entry.path().display()
            ));
        }
    }
    Ok(())
}
