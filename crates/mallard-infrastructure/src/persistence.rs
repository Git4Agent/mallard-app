//! Bounded, atomic local persistence.
//!
//! Every file lives below `~/.mallard`. All read-modify-write operations
//! share a process lock and revision check so concurrent Tauri commands
//! cannot silently lose changes.

use mallard_domain::{
    derive_origin_host_token, derive_origin_replica_token, generated_named_id, BundleId,
    ConversationBindings, HostId, HostIdentity, LocalProjectId, MachineProjectState,
    Materializations, OriginHostToken, OriginReplicaToken, ProjectSetupDraft, ReplicaId,
    SetupDraftId, SetupTransaction, StorageConfig, StorageId, StorageSetupDraft,
    StorageSetupDraftId, SyncConfig, CONVERSATION_BINDINGS_SCHEMA, HOST_IDENTITY_SCHEMA,
    MACHINE_PROJECT_SCHEMA, MAX_SETUP_DRAFTS, MAX_STORAGE_SETUP_DRAFTS,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BINDINGS_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MATERIALIZATIONS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SETUP_DRAFT_BYTES: u64 = 4 * 1024 * 1024;
// A storage draft is a handful of ≤4096-byte validated strings; 128 KiB
// never rejects a valid draft while bounding read amplification for
// corrupt files far below the project-draft cap.
const MAX_STORAGE_SETUP_DRAFT_BYTES: u64 = 128 * 1024;
const MAX_SETUP_TRANSACTION_BYTES: u64 = 8 * 1024 * 1024;
const MAX_HOST_IDENTITY_BYTES: u64 = 16 * 1024;
// Rotation journals embed every bounded host-scoped document. They are stored
// as compact JSON to avoid nested pretty-print expansion; the additional
// headroom covers wrapper fields and bounded default growth.
const MAX_HOST_ROTATION_BYTES: u64 = 192 * 1024 * 1024;

const MAX_CONVERSATION_BINDINGS_BYTES: u64 = 64 * 1024 * 1024;
const HOST_ROTATION_SCHEMA: u32 = 1;

static PERSISTENCE_LOCK: Mutex<()> = Mutex::new(());

/// The root tail layout codec the registered adapter declares for one
/// provider; conversation bindings of unregistered providers carry none.
fn tail_layout_codec_for(provider: &mallard_domain::ProviderKey) -> Option<&'static str> {
    crate::providers::AgentProviderRegistry::production().tail_layout_codec(provider)
}

/// Whether a focused config transaction changed the document. Returning
/// [`Unchanged`](ConfigMutation::Unchanged) preserves the current revision and
/// avoids an unnecessary atomic rewrite, which is required for idempotent
/// command retries.
pub enum ConfigMutation<T> {
    Unchanged(T),
    Changed(T),
}

/// Separates a caller's transactional decision from persistence failures.
/// Application services can therefore retain typed conflict errors without
/// parsing repository error strings.
pub enum ConfigTransactionError<E> {
    Mutation(E),
    Persistence(String),
}

#[derive(Serialize, Deserialize)]
struct ProjectBundleTransaction {
    config: SyncConfig,
    machine: MachineProjectState,
}

/// Exact machine-local state removed by an explicit project sync reset.
/// Project registration, provider/storage configuration, user recipes, local
/// files, and remote objects are deliberately outside this result and outside
/// the mutation.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectSyncMetadataResetResult {
    pub local_project_id: LocalProjectId,
    pub config_revision: u64,
    pub links_reset: u64,
    pub scope_bases_removed: u64,
    pub conversation_checkpoints_removed: u64,
    pub conversation_bindings_removed: u64,
    pub materializations_removed: u64,
}

/// Destination-scoped derived state forgotten after an explicit destructive
/// storage clear. Project registrations, links, recipes, bindings, provider
/// files, and the storage configuration remain in place.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageSyncMetadataResetResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub links_reset: u64,
    pub scope_bases_removed: u64,
    pub conversation_acceptances_removed: u64,
    pub materializations_removed: u64,
}

/// What lives under `~/.mallard` right now, measured lock-free: the readout is
/// a plain measurement and must stay readable while a sync holds the
/// persistence lock. `backups_bytes` is called out separately because the
/// backups subtree holds the only local copies of files Pull replaced — the
/// one part of a wipe the confirm dialog has to name.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalStateSummary {
    pub root_path: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub backups_bytes: u64,
}

/// Outcome of deleting the whole metadata root. `existed: false` means there
/// was nothing to delete — the wipe is idempotent and that is a success.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalStateWipeResult {
    pub existed: bool,
    pub files_removed: u64,
    pub bytes_removed: u64,
}

/// Project-facing machine state read under one persistence lock. The config
/// document is loaded once and validates both dependent documents.
#[derive(Clone, Debug)]
pub struct LocalProjectStateSnapshot {
    pub host_identity: HostIdentity,
    pub config: SyncConfig,
    pub bindings: MachineProjectState,
    pub materializations: Materializations,
}

/// Activity-facing machine state read and cross-validated under one
/// persistence lock.
#[derive(Debug)]
pub struct LocalProjectActivityStateSnapshot {
    pub host_identity: HostIdentity,
    pub config: SyncConfig,
    pub bindings: MachineProjectState,
    pub conversation_bindings: ConversationBindings,
    pub materializations: Materializations,
}

/// One before-state and one final after-state for a published Push batch.
#[derive(Debug)]
pub struct ConversationPublicationState {
    pub sync_config_before: SyncConfig,
    pub sync_config_after: SyncConfig,
    pub bindings_before: ConversationBindings,
    pub bindings_after: ConversationBindings,
}

impl ConversationPublicationState {
    pub(crate) fn validate(
        &self,
        current: &LocalProjectActivityStateSnapshot,
    ) -> Result<(), String> {
        if current.config != self.sync_config_before {
            return Err("sync config changed during apply".to_string());
        }
        if current.conversation_bindings != self.bindings_before {
            return Err("conversation bindings changed during apply".to_string());
        }
        if self.sync_config_after.revision != current.config.revision.saturating_add(1)
            || self.bindings_after.revision
                != current.conversation_bindings.revision.saturating_add(1)
        {
            return Err("conversation Push after-state has an invalid revision".to_string());
        }
        self.sync_config_after.validate()?;
        self.bindings_after.validate(
            &current.host_identity,
            &current.bindings,
            &self.sync_config_after,
            &tail_layout_codec_for,
        )
    }
}

/// Current state for one Pull installation. The persistence lock excludes config
/// writers as well as other apply operations. Callers must not re-enter
/// repository methods that acquire this lock while the installation is alive.
pub struct LockedConversationPull<'a> {
    repository: &'a LocalStateRepository,
    _guard: PersistenceGuard,
    state: LocalProjectActivityStateSnapshot,
    usable: bool,
}

impl LockedConversationPull<'_> {
    pub fn current(&self) -> Result<&LocalProjectActivityStateSnapshot, String> {
        if !self.usable {
            return Err("conversation Pull state is uncertain after a failed commit".to_string());
        }
        Ok(&self.state)
    }

    /// Recover the durable before-state for the next session after a partial
    /// write. The lock stays held, so revision changes here can only come from
    /// this installation. Do not reuse the failed candidate's in-memory state.
    pub fn reload_after_failed_commit(&mut self) -> Result<(), String> {
        self.state = self
            .repository
            .load_project_activity_state_snapshot_unlocked()?;
        self.usable = true;
        Ok(())
    }

    pub fn store_path_origins(
        &self,
        owner: crate::session_path_origins::PathOriginsOwner,
        origins: &crate::session_path_origins::PathOrigins,
    ) -> Result<mallard_domain::PathOriginsRef, String> {
        self.current()?;
        self.repository.store_path_origins_unlocked(owner, origins)
    }

    pub fn derive_origin_tokens(
        &self,
        bundle_id: &BundleId,
        replica_id: &ReplicaId,
    ) -> Result<(OriginHostToken, OriginReplicaToken), String> {
        let identity = &self.current()?.host_identity;
        Ok((
            derive_origin_host_token(&identity.host_id, bundle_id)?,
            derive_origin_replica_token(&identity.host_id, bundle_id, replica_id)?,
        ))
    }

    pub(crate) fn repository(&self) -> &LocalStateRepository {
        self.repository
    }

    /// Keep the existing per-session write order. After a write error the
    /// caller reloads durable state: an atomic rename may already have landed.
    pub(crate) fn commit(
        &mut self,
        config: SyncConfig,
        bindings: ConversationBindings,
        record_receipt: impl FnOnce(&mut Materializations) -> Result<(), String>,
    ) -> Result<(), String> {
        let current = self.current()?;
        if config.revision != current.config.revision.saturating_add(1)
            || bindings.revision != current.conversation_bindings.revision.saturating_add(1)
        {
            return Err("conversation Pull after-state has an invalid revision".to_string());
        }
        config.validate()?;
        bindings.validate(
            &current.host_identity,
            &current.bindings,
            &current.config,
            &tail_layout_codec_for,
        )?;

        self.usable = false;
        let repository = self.repository;
        write_json_atomic(
            repository.root(),
            &repository.conversation_bindings_path(),
            &bindings,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )?;
        write_json_atomic(
            repository.root(),
            &repository.config_path(),
            &config,
            MAX_CONFIG_BYTES,
        )?;
        let materializations = &mut self.state.materializations;
        record_receipt(materializations)?;
        materializations.schema = mallard_domain::LOCAL_SCHEMA;
        materializations.revision = materializations.revision.saturating_add(1);
        materializations.validate(&config)?;
        write_json_atomic(
            repository.root(),
            &repository.materializations_path(),
            materializations,
            MAX_MATERIALIZATIONS_BYTES,
        )?;
        self.state.config = config;
        self.state.conversation_bindings = bindings;
        self.usable = true;
        Ok(())
    }
}

pub(crate) struct PersistenceGuard {
    _process: std::sync::MutexGuard<'static, ()>,
    _file: fs::File,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct HostRotationQuarantine {
    #[serde(default)]
    machine_projects: bool,
    #[serde(default)]
    conversation_bindings: bool,
    #[serde(default)]
    materializations: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct HostRotationTransaction {
    schema: u32,
    next_identity: HostIdentity,
    machine_projects: MachineProjectState,
    conversation_bindings: ConversationBindings,
    materializations: Materializations,
    #[serde(default)]
    quarantine: HostRotationQuarantine,
    created_at: u64,
}

#[derive(Debug)]
enum BoundedJsonReadError {
    Corrupt(String),
    Io(String),
}

impl BoundedJsonReadError {
    fn into_message(self) -> String {
        match self {
            Self::Corrupt(message) | Self::Io(message) => message,
        }
    }
}

/// Held across host-sensitive workflows (restore apply, setup,
/// publication, and rotation). The OS lock coordinates separate Mallard
/// processes; dropping the file releases it on every supported platform.
#[derive(Debug)]
pub struct HostOperationGuard {
    _file: fs::File,
}

impl Drop for HostOperationGuard {
    fn drop(&mut self) {
        // Explicitly unlock before close. A child process spawned while this
        // guard is held may briefly inherit the descriptor before exec closes
        // it; unlocking prevents that descriptor extending the operation lock.
        let _ = self._file.unlock();
    }
}

#[derive(Clone, Debug)]
pub struct LocalStateRepository {
    root: PathBuf,
}

impl LocalStateRepository {
    /// Resolve the machine-global metadata directory from the user's home.
    pub fn from_user_home() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or_else(|| "cannot find home directory".to_string())?;
        Self::from_home_dir(home)
    }

    /// Construct from a home directory. Tests and non-Tauri helpers use this
    /// to exercise exactly the production path without touching the real home.
    pub fn from_home_dir(home: impl Into<PathBuf>) -> Result<Self, String> {
        let home = home.into();
        if !home.is_absolute() {
            return Err(format!(
                "home directory must be absolute: '{}'",
                home.display()
            ));
        }
        let repository = Self {
            root: home.join(".mallard"),
        };
        repository.ensure_root()?;
        Ok(repository)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn acquire_host_operation(&self) -> Result<HostOperationGuard, String> {
        let path = self.root.join("host_operation.lock");
        ensure_no_symlinks(&self.root, &path)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(&path)
            .map_err(|error| format!("open host-operation lock: {}", error))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect host-operation lock: {}", error))?;
        if !metadata.file_type().is_file() {
            return Err("host-operation lock is not a regular file".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::fs::PermissionsExt;
            if metadata.nlink() != 1 {
                return Err("host-operation lock has unsafe hard links".to_string());
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("secure host-operation lock: {}", error))?;
        }
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                "another host-sensitive operation is already in progress".to_string()
            }
            std::fs::TryLockError::Error(error) => {
                format!("lock host-sensitive operation: {}", error)
            }
        })?;
        Ok(HostOperationGuard { _file: file })
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("sync_config.json")
    }

    pub fn machine_projects_path(&self) -> PathBuf {
        self.root.join("machine_projects.json")
    }

    pub fn host_identity_path(&self) -> PathBuf {
        self.root.join("host_identity.json")
    }

    fn host_rotation_path(&self) -> PathBuf {
        self.root.join("host_rotation.json")
    }

    pub fn conversation_bindings_path(&self) -> PathBuf {
        self.root.join("conversation_bindings.json")
    }

    pub fn materializations_path(&self) -> PathBuf {
        self.root.join("materializations.json")
    }

    pub fn backups_dir(&self) -> Result<PathBuf, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.root.join("backups");
        self.ensure_directory(&path)?;
        Ok(path)
    }

    /// Measure everything under the metadata root. Deliberately lock-free: a
    /// racy readout is fine for a settings row, and it must not block behind a
    /// running sync. A missing root reports zeros rather than an error.
    pub fn local_state_summary(&self) -> Result<LocalStateSummary, String> {
        let (file_count, total_bytes) = walk_local_state(&self.root)?;
        let (_, backups_bytes) = walk_local_state(&self.root.join("backups"))?;
        Ok(LocalStateSummary {
            root_path: self.root.display().to_string(),
            file_count,
            total_bytes,
            backups_bytes,
        })
    }

    /// Delete the whole metadata root — sync state, host identity, saved
    /// plans, backups, and logs. The caller restarts the app afterwards; the
    /// next repository construction recreates a fresh root via `ensure_root`,
    /// which is why nothing here recreates it.
    pub fn wipe_local_state(&self) -> Result<LocalStateWipeResult, String> {
        // Insurance against a future refactor pointing `root` anywhere else:
        // this method deletes recursively and must never aim at a home dir.
        if self.root.file_name() != Some(std::ffi::OsStr::new(".mallard")) {
            return Err(format!(
                "refusing to delete '{}': not a .mallard metadata root",
                self.root.display()
            ));
        }
        // Missing-root check BEFORE any lock: both guard files live inside the
        // root, so acquiring first would recreate state and break idempotence.
        match fs::symlink_metadata(&self.root) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(format!(
                    "metadata directory '{}' is not a real directory",
                    self.root.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LocalStateWipeResult {
                    existed: false,
                    files_removed: 0,
                    bytes_removed: 0,
                });
            }
            Err(error) => {
                return Err(format!(
                    "inspect metadata directory '{}': {}",
                    self.root.display(),
                    error
                ));
            }
        }
        // Same order as every host-sensitive mutation: host operation first
        // (try-lock — a running push/pull makes the reset fail fast instead of
        // queueing a deletion behind it), persistence second.
        let _operation = self.acquire_host_operation()?;
        let _guard = persistence_guard(&self.root)?;
        let (files_removed, bytes_removed) = walk_local_state(&self.root)?;
        // Once our own lock files are unlinked mid-traversal another process
        // can recreate entries; a bounded retry absorbs that without ever
        // resurrecting old data.
        let mut attempts = 0;
        loop {
            match fs::remove_dir_all(&self.root) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) if attempts < 2 => attempts += 1,
                Err(error) => {
                    return Err(format!(
                        "delete metadata directory '{}': {}",
                        self.root.display(),
                        error
                    ));
                }
            }
        }
        Ok(LocalStateWipeResult {
            existed: true,
            files_removed,
            bytes_removed,
        })
    }

    /// Load the installation identity. A missing or damaged identity is
    /// replaced automatically: the app registers a fresh host on open
    /// instead of surfacing a recovery decision.
    pub fn load_or_create_host_identity(&self) -> Result<HostIdentity, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()
    }

    /// Publish a fresh identity and rebase every host-scoped document under
    /// it inside one recoverable rotation journal. Automatic registration
    /// quarantines damaged host-scoped documents and replaces them.
    fn replace_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        self.validate_setup_transactions_unlocked()?;

        let config = self.load_config_unlocked()?;
        let mut quarantine = HostRotationQuarantine::default();
        let next = self.prepare_host_identity_unlocked()?;
        let mut machine = match self.load_bindings_document_unlocked(&config) {
            Ok(machine) => machine,
            Err(_) => {
                quarantine.machine_projects = true;
                MachineProjectState::default()
            }
        };
        let mut conversations = match read_json_bounded::<ConversationBindings>(
            &self.root,
            &self.conversation_bindings_path(),
            MAX_CONVERSATION_BINDINGS_BYTES,
        ) {
            Ok(Some(bindings)) => bindings,
            Ok(None) => ConversationBindings::empty(next.host_id.clone()),
            Err(_) => {
                quarantine.conversation_bindings = true;
                ConversationBindings::empty(next.host_id.clone())
            }
        };
        // Salvaged alias rows must not wedge automatic registration.
        let mut seen = BTreeSet::new();
        conversations.origin_aliases.retain(|alias| {
            alias.validate().is_ok()
                && seen.insert((alias.bundle_id.clone(), alias.origin_host_token.clone()))
        });
        conversations
            .origin_aliases
            .truncate(mallard_domain::MAX_ORIGIN_ALIASES);
        let mut materializations = match self.load_materializations_unlocked(&config) {
            Ok(materializations) => materializations,
            Err(_) => {
                quarantine.materializations = true;
                Materializations::default()
            }
        };
        let timestamp = next.updated_at;
        let mut replica_map = BTreeMap::new();
        for binding in &mut machine.bindings {
            let previous_replica = binding.replica_id.clone();
            let next_replica = ReplicaId::parse(generated_named_id("replica")?)?;
            binding.host_id = next.host_id.clone();
            binding.replica_id = next_replica.clone();
            binding.state = mallard_domain::BindingState::Detached;
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = binding.updated_at.max(timestamp);
            replica_map.insert(previous_replica, next_replica);
        }
        machine.schema = MACHINE_PROJECT_SCHEMA;
        machine.revision = machine.revision.saturating_add(1);
        machine.validate(&config)?;

        conversations.host_id = next.host_id.clone();
        conversations.revision = conversations.revision.saturating_add(1);
        let mut rotated_bindings = Vec::new();
        for mut binding in std::mem::take(&mut conversations.bindings) {
            if binding.validate().is_err() {
                continue;
            }
            let Some(next_replica) = replica_map.get(&binding.replica_id) else {
                continue;
            };
            let origins = binding
                .path_origins_ref
                .as_ref()
                .map(|_| self.load_path_origins(&binding))
                .transpose()?;
            binding.host_id = next.host_id.clone();
            binding.replica_id = next_replica.clone();
            if let Some(origins) = origins {
                binding.path_origins_ref = Some(self.store_path_origins_unlocked(
                    crate::session_path_origins::PathOriginsOwner::for_binding(&binding)?,
                    &origins,
                )?);
            }
            binding.materialization_state = if binding.provider_thread_id.is_some() {
                mallard_domain::ConversationMaterializationState::Pending
            } else {
                mallard_domain::ConversationMaterializationState::StoredOnly
            };
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = binding.updated_at.max(binding.created_at).max(timestamp);
            rotated_bindings.push(binding);
        }
        conversations.bindings = rotated_bindings;
        conversations.validate(&next, &machine, &config, &tail_layout_codec_for)?;

        for record in &mut materializations.records {
            record.status = mallard_domain::MaterializationStatus::Detached;
        }
        materializations.revision = materializations.revision.saturating_add(1);
        materializations.validate(&config)?;

        let transaction = HostRotationTransaction {
            schema: HOST_ROTATION_SCHEMA,
            next_identity: next.clone(),
            machine_projects: machine,
            conversation_bindings: conversations,
            materializations,
            quarantine,
            created_at: timestamp,
        };
        self.validate_host_rotation_payload_unlocked(&transaction, &config)?;
        write_json_atomic_compact(
            &self.root,
            &self.host_rotation_path(),
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        )?;
        self.commit_host_rotation_unlocked(&transaction)?;
        Ok(next)
    }

    pub fn derive_origin_tokens(
        &self,
        bundle_id: &BundleId,
        replica_id: &ReplicaId,
    ) -> Result<(OriginHostToken, OriginReplicaToken), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        Ok((
            derive_origin_host_token(&identity.host_id, bundle_id)?,
            derive_origin_replica_token(&identity.host_id, bundle_id, replica_id)?,
        ))
    }

    #[cfg(test)]
    pub fn derive_origin_host_token(
        &self,
        bundle_id: &BundleId,
    ) -> Result<OriginHostToken, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        derive_origin_host_token(&identity.host_id, bundle_id)
    }

    pub fn load_conversation_bindings(&self) -> Result<ConversationBindings, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        self.load_conversation_bindings_document_unlocked(&config, &machine, &identity)
    }

    pub fn store_path_origins(
        &self,
        owner: crate::session_path_origins::PathOriginsOwner,
        origins: &crate::session_path_origins::PathOrigins,
    ) -> Result<mallard_domain::PathOriginsRef, String> {
        let _guard = persistence_guard(&self.root)?;
        self.store_path_origins_unlocked(owner, origins)
    }

    fn store_path_origins_unlocked(
        &self,
        owner: crate::session_path_origins::PathOriginsOwner,
        origins: &crate::session_path_origins::PathOrigins,
    ) -> Result<mallard_domain::PathOriginsRef, String> {
        if !origins
            .sources
            .get(&owner.provider_thread_id)
            .is_some_and(|records| !records.is_empty())
        {
            return Err("path origins do not cover their physical owner".into());
        }
        let directory = self.root.join("path_origins");
        ensure_no_symlinks(&self.root, &directory)?;
        create_metadata_root(&directory)?;
        let document = crate::session_path_origins::PathOriginsFile {
            schema: 1,
            codec: crate::session_path_map::PATH_MAP_CODEC_V1.to_string(),
            owner,
            origins: origins.clone(),
        };
        let bytes = serde_json::to_vec(&document).map_err(|_| "serialize path origins")?;
        let reference = mallard_domain::PathOriginsRef {
            sha256: mallard_domain::sha256_hex(&bytes),
            size_bytes: bytes.len() as u64,
        };
        reference.validate()?;
        let path = directory.join(format!("{}.json", reference.sha256));
        if ensure_no_symlinks(&self.root, &path)?.is_some() {
            read_path_origin_bytes(&self.root, &path, &reference)?;
        } else {
            write_bytes_atomic(&self.root, &path, &bytes, 128 * 1024 * 1024)?;
        }
        Ok(reference)
    }

    pub fn load_path_origins(
        &self,
        binding: &mallard_domain::LocalConversationBinding,
    ) -> Result<crate::session_path_origins::PathOrigins, String> {
        let reference = binding
            .path_origins_ref
            .as_ref()
            .ok_or("tracked thread has no path origins")?;
        let owner = crate::session_path_origins::PathOriginsOwner::for_binding(binding)?;
        let path = self
            .root
            .join("path_origins")
            .join(format!("{}.json", reference.sha256));
        let bytes = read_path_origin_bytes(&self.root, &path, reference)?;
        let file: crate::session_path_origins::PathOriginsFile =
            serde_json::from_slice(&bytes).map_err(|_| "invalid path-origin file")?;
        if file.schema != 1
            || file.codec != crate::session_path_map::PATH_MAP_CODEC_V1
            || file.owner != owner
        {
            return Err("path origins have the wrong owner or codec".to_string());
        }
        if !file
            .origins
            .sources
            .get(&owner.provider_thread_id)
            .is_some_and(|records| !records.is_empty())
        {
            return Err("path origins do not cover their physical owner".into());
        }
        Ok(file.origins)
    }

    pub fn mutate_conversation_bindings<T>(
        &self,
        mutate: impl FnOnce(&mut ConversationBindings) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        let mut bindings =
            self.load_conversation_bindings_document_unlocked(&config, &machine, &identity)?;
        let result = mutate(&mut bindings)?;
        bindings.schema = CONVERSATION_BINDINGS_SCHEMA;
        bindings.revision = bindings.revision.saturating_add(1);
        bindings.host_id = identity.host_id.clone();
        bindings.validate(&identity, &machine, &config, &tail_layout_codec_for)?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            &bindings,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )?;
        Ok(result)
    }

    /// Commit the published Push batch under one persistence lock. Each state
    /// document is written once; receipts are included in one final update.
    pub(crate) fn commit_conversation_publication(
        &self,
        state: &ConversationPublicationState,
        record_receipts: impl FnOnce(&mut Materializations) -> Result<(), String>,
    ) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        let mut current = self.load_project_activity_state_snapshot_unlocked()?;
        state.validate(&current)?;
        record_receipts(&mut current.materializations)?;
        current.materializations.schema = mallard_domain::LOCAL_SCHEMA;
        current.materializations.revision = current.materializations.revision.saturating_add(1);
        current
            .materializations
            .validate(&state.sync_config_after)?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            &state.bindings_after,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )?;
        write_json_atomic(
            &self.root,
            &self.config_path(),
            &state.sync_config_after,
            MAX_CONFIG_BYTES,
        )?;
        write_json_atomic(
            &self.root,
            &self.materializations_path(),
            &current.materializations,
            MAX_MATERIALIZATIONS_BYTES,
        )
    }

    /// Install one apply-time whole-document bindings after-state exactly as
    /// built: validated, atomically written, and never re-revisioned. The
    /// caller proves the current document equals the expected before-state.
    pub fn install_conversation_bindings_snapshot(
        &self,
        document: &ConversationBindings,
    ) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        document.validate(&identity, &machine, &config, &tail_layout_codec_for)?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            document,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )
    }

    /// Install one apply-time whole-document `SyncConfig` after-state exactly
    /// as built: validated, atomically written, and never re-revisioned. The
    /// caller proves the current document equals the expected before-state,
    /// so an exact replacement cannot overwrite intervening config changes.
    /// Ordinary service mutations use revisioned `transact_config` calls.
    pub fn install_config_snapshot(&self, document: &SyncConfig) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        document.validate()?;
        write_json_atomic(&self.root, &self.config_path(), document, MAX_CONFIG_BYTES)
    }

    pub fn load_config(&self) -> Result<SyncConfig, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_config_unlocked()
    }

    pub fn load_project_state_snapshot(&self) -> Result<LocalProjectStateSnapshot, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let bindings = self.load_bindings_unlocked(&config, &identity)?;
        let materializations = self.load_materializations_unlocked(&config)?;
        Ok(LocalProjectStateSnapshot {
            host_identity: identity,
            config,
            bindings,
            materializations,
        })
    }

    pub fn load_project_activity_state_snapshot(
        &self,
    ) -> Result<LocalProjectActivityStateSnapshot, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_project_activity_state_snapshot_unlocked()
    }

    fn load_project_activity_state_snapshot_unlocked(
        &self,
    ) -> Result<LocalProjectActivityStateSnapshot, String> {
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let bindings = self.load_bindings_unlocked(&config, &identity)?;
        let conversation_bindings =
            self.load_conversation_bindings_document_unlocked(&config, &bindings, &identity)?;
        let materializations = self.load_materializations_unlocked(&config)?;
        Ok(LocalProjectActivityStateSnapshot {
            host_identity: identity,
            config,
            bindings,
            conversation_bindings,
            materializations,
        })
    }

    /// Lock one local installation after transcript preparation. The caller
    /// also holds the host-operation guard and checks the batch's expected
    /// revisions against this freshly loaded state before installing files.
    pub fn lock_conversation_pull(&self) -> Result<LockedConversationPull<'_>, String> {
        let guard = persistence_guard(&self.root)?;
        Ok(LockedConversationPull {
            repository: self,
            _guard: guard,
            state: self.load_project_activity_state_snapshot_unlocked()?,
            usable: true,
        })
    }

    /// Full-document optimistic save used by storage/settings UI.  The
    /// caller must submit the revision it read; the returned value carries
    /// the next revision.
    pub fn save_config(&self, mut config: SyncConfig) -> Result<SyncConfig, String> {
        let _guard = persistence_guard(&self.root)?;
        let current = self.load_config_unlocked()?;
        if config.revision != current.revision {
            return Err(format!(
                "project-sync config changed (expected revision {}, current {})",
                config.revision, current.revision
            ));
        }
        config.schema = mallard_domain::LOCAL_SCHEMA;
        config.revision = current.revision.saturating_add(1);
        config.validate()?;
        write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)?;
        Ok(config)
    }

    /// Transactional config mutation for focused project/link/recipe
    /// commands.  The closure runs while the process persistence lock is held.
    pub fn mutate_config<T>(
        &self,
        mutate: impl FnOnce(&mut SyncConfig) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let mut config = self.load_config_unlocked()?;
        let result = mutate(&mut config)?;
        config.schema = mallard_domain::LOCAL_SCHEMA;
        config.revision = config.revision.saturating_add(1);
        config.validate()?;
        write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)?;
        Ok(result)
    }

    /// Run one focused config transaction and return the exact document that
    /// was observed or committed while `persistence.lock` is still held.
    ///
    /// Network work must happen before entering this method. The closure is
    /// intentionally limited to an in-memory decision so the cross-process
    /// persistence lock is never held across remote I/O.
    pub fn transact_config<T, E>(
        &self,
        mutate: impl FnOnce(&mut SyncConfig) -> Result<ConfigMutation<T>, E>,
    ) -> Result<(T, SyncConfig), ConfigTransactionError<E>> {
        let _guard = persistence_guard(&self.root).map_err(ConfigTransactionError::Persistence)?;
        let mut config = self
            .load_config_unlocked()
            .map_err(ConfigTransactionError::Persistence)?;
        let decision = mutate(&mut config).map_err(ConfigTransactionError::Mutation)?;
        match decision {
            ConfigMutation::Unchanged(result) => Ok((result, config)),
            ConfigMutation::Changed(result) => {
                config.schema = mallard_domain::LOCAL_SCHEMA;
                config.revision = config.revision.saturating_add(1);
                config
                    .validate()
                    .map_err(ConfigTransactionError::Persistence)?;
                write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)
                    .map_err(ConfigTransactionError::Persistence)?;
                Ok((result, config))
            }
        }
    }

    pub fn load_bindings(&self) -> Result<MachineProjectState, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        self.load_bindings_unlocked(&config, &identity)
    }

    /// Change a project's bundle without replacing its replica or conversation
    /// identities. The journal completes both document writes after a crash.
    /// The caller holds the host-operation guard and performs remote checks first.
    pub fn mutate_project_bundle(
        &self,
        project_id: &LocalProjectId,
        expected_config_revision: u64,
        mutate: impl FnOnce(&mut SyncConfig) -> Result<(), String>,
    ) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let mut config = self.load_config_unlocked()?;
        if config.revision != expected_config_revision {
            return Err("project configuration changed while connecting remote storage".into());
        }
        let mut machine = self.load_bindings_unlocked(&config, &identity)?;
        mutate(&mut config)?;
        config.revision = config.revision.saturating_add(1);
        let project = config.project(project_id).ok_or("project disappeared")?;
        for binding in machine
            .bindings
            .iter_mut()
            .filter(|binding| &binding.local_project_id == project_id)
        {
            binding.bundle_id = project.bundle_id.clone();
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = project.updated_at;
        }
        machine.revision = machine.revision.saturating_add(1);
        config.validate()?;
        machine.validate(&config)?;
        self.load_conversation_bindings_document_unlocked(&config, &machine, &identity)?;
        self.load_materializations_unlocked(&config)?;
        let transaction = ProjectBundleTransaction { config, machine };
        self.validate_project_bundle_transaction(&transaction)?;
        write_json_atomic(
            &self.root,
            &self.root.join("project_bundle_transaction.json"),
            &transaction,
            2 * (MAX_CONFIG_BYTES + MAX_BINDINGS_BYTES),
        )?;
        self.recover_project_bundle_unlocked()
    }

    fn validate_project_bundle_transaction(
        &self,
        transaction: &ProjectBundleTransaction,
    ) -> Result<(), String> {
        transaction.config.validate()?;
        transaction.machine.validate(&transaction.config)?;
        // Check individual write limits before making the journal durable.
        if serde_json::to_vec_pretty(&transaction.config)
            .map_err(|e| e.to_string())?
            .len() as u64
            + 1
            > MAX_CONFIG_BYTES
            || serde_json::to_vec_pretty(&transaction.machine)
                .map_err(|e| e.to_string())?
                .len() as u64
                + 1
                > MAX_BINDINGS_BYTES
        {
            return Err("project bundle transaction exceeds document limits".into());
        }
        Ok(())
    }

    fn recover_project_bundle_unlocked(&self) -> Result<(), String> {
        let path = self.root.join("project_bundle_transaction.json");
        let Some(transaction) = read_json_bounded::<ProjectBundleTransaction>(
            &self.root,
            &path,
            2 * (MAX_CONFIG_BYTES + MAX_BINDINGS_BYTES),
        )?
        else {
            return Ok(());
        };
        self.validate_project_bundle_transaction(&transaction)?;
        let identity = self
            .load_host_identity_document_classified_unlocked()
            .map_err(BoundedJsonReadError::into_message)?
            .ok_or("project bundle transaction has no host identity")?;
        if transaction
            .machine
            .bindings
            .iter()
            .any(|binding| binding.host_id != identity.host_id)
        {
            return Err("project bundle transaction belongs to a different host".into());
        }
        self.load_conversation_bindings_document_unlocked(
            &transaction.config,
            &transaction.machine,
            &identity,
        )?;
        self.load_materializations_unlocked(&transaction.config)?;
        write_json_atomic(
            &self.root,
            &self.machine_projects_path(),
            &transaction.machine,
            MAX_BINDINGS_BYTES,
        )?;
        write_json_atomic(
            &self.root,
            &self.config_path(),
            &transaction.config,
            MAX_CONFIG_BYTES,
        )?;
        fs::remove_file(&path).map_err(|e| format!("retire project bundle transaction: {e}"))?;
        sync_directory(&self.root)
    }

    pub fn mutate_bindings<T>(
        &self,
        mutate: impl FnOnce(&SyncConfig, &mut MachineProjectState) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let mut bindings = self.load_bindings_unlocked(&config, &identity)?;
        let result = mutate(&config, &mut bindings)?;
        bindings.schema = MACHINE_PROJECT_SCHEMA;
        bindings.revision = bindings.revision.saturating_add(1);
        if bindings
            .bindings
            .iter()
            .any(|binding| binding.host_id != identity.host_id)
        {
            return Err("project binding belongs to a different host".to_string());
        }
        bindings.validate(&config)?;
        // Machine bindings and conversation bindings are one logical scope:
        // changing or removing a replica/profile must not strand durable
        // conversation identities in a document that can no longer load.
        // Validate the existing conversation document against the prospective
        // machine state before publishing either change.
        self.load_conversation_bindings_document_unlocked(&config, &bindings, &identity)?;
        write_json_atomic(
            &self.root,
            &self.machine_projects_path(),
            &bindings,
            MAX_BINDINGS_BYTES,
        )?;
        Ok(result)
    }

    pub fn load_materializations(&self) -> Result<Materializations, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        self.load_materializations_unlocked(&config)
    }

    pub fn mutate_materializations<T>(
        &self,
        mutate: impl FnOnce(&SyncConfig, &mut Materializations) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let mut materializations = self.load_materializations_unlocked(&config)?;
        let result = mutate(&config, &mut materializations)?;
        materializations.schema = mallard_domain::LOCAL_SCHEMA;
        materializations.revision = materializations.revision.saturating_add(1);
        materializations.validate(&config)?;
        write_json_atomic(
            &self.root,
            &self.materializations_path(),
            &materializations,
            MAX_MATERIALIZATIONS_BYTES,
        )?;
        Ok(result)
    }

    /// Forget every derived sync decision and local conversation binding for
    /// one configured project, including bindings with restored path origins.
    /// Retain project setup, immutable origin files, and every user-owned byte.
    ///
    /// All prospective documents are validated before the first write. Each
    /// write is an atomic replacement and each intermediate state is valid,
    /// so a process crash is repaired by rerunning this idempotent reset.
    pub fn reset_project_sync_metadata(
        &self,
        local_project_id: &LocalProjectId,
        expected_project_revision: u64,
    ) -> Result<ProjectSyncMetadataResetResult, String> {
        let _operation = self.acquire_host_operation()?;
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let mut config = self.load_config_unlocked()?;
        let project = config
            .project(local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
        if project.revision != expected_project_revision {
            return Err(format!(
                "project changed (expected revision {}, current {})",
                expected_project_revision, project.revision
            ));
        }

        let machine = self.load_bindings_unlocked(&config, &identity)?;
        let mut conversation_bindings =
            self.load_conversation_bindings_document_unlocked(&config, &machine, &identity)?;
        let mut materializations = self.load_materializations_unlocked(&config)?;

        let mut links_reset = 0_u64;
        let mut scope_bases_removed = 0_u64;
        let mut conversation_checkpoints_removed = 0_u64;
        let mut config_changed = false;
        for link in config
            .links
            .iter_mut()
            .filter(|link| &link.local_project_id == local_project_id)
        {
            links_reset = links_reset.saturating_add(1);
            scope_bases_removed = scope_bases_removed.saturating_add(link.scope_bases.len() as u64);
            conversation_checkpoints_removed = conversation_checkpoints_removed.saturating_add(
                link.conversation_checkpoints
                    .values()
                    .map(|threads| threads.len() as u64)
                    .sum::<u64>(),
            );
            config_changed |=
                !link.scope_bases.is_empty() || !link.conversation_checkpoints.is_empty();
            link.scope_bases.clear();
            link.conversation_checkpoints.clear();
        }

        let conversation_binding_count = conversation_bindings.bindings.len();
        conversation_bindings
            .bindings
            .retain(|binding| &binding.local_project_id != local_project_id);
        let conversation_bindings_removed =
            (conversation_binding_count - conversation_bindings.bindings.len()) as u64;

        let materialization_count = materializations.records.len();
        materializations
            .records
            .retain(|record| &record.local_project_id != local_project_id);
        let materializations_removed =
            (materialization_count - materializations.records.len()) as u64;

        if config_changed {
            config.revision = config.revision.saturating_add(1);
        }
        if conversation_bindings_removed > 0 {
            conversation_bindings.revision = conversation_bindings.revision.saturating_add(1);
        }
        if materializations_removed > 0 {
            materializations.revision = materializations.revision.saturating_add(1);
        }
        config.validate()?;
        machine.validate(&config)?;
        conversation_bindings.validate(&identity, &machine, &config, &tail_layout_codec_for)?;
        materializations.validate(&config)?;

        // Checkpoints are detached before aliases so a crash never leaves an
        // accepted destination cursor pointing at a forgotten provider row.
        if config_changed {
            write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)?;
        }
        if conversation_bindings_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.conversation_bindings_path(),
                &conversation_bindings,
                MAX_CONVERSATION_BINDINGS_BYTES,
            )?;
        }
        if materializations_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.materializations_path(),
                &materializations,
                MAX_MATERIALIZATIONS_BYTES,
            )?;
        }
        Ok(ProjectSyncMetadataResetResult {
            local_project_id: local_project_id.clone(),
            config_revision: config.revision,
            links_reset,
            scope_bases_removed,
            conversation_checkpoints_removed,
            conversation_bindings_removed,
            materializations_removed,
        })
    }

    /// Pin one exact configured destination, clear its remote Mallard data,
    /// then forget only destination-scoped local sync decisions. The host and
    /// persistence locks intentionally span the remote callback: a destructive
    /// clear must not race a credential/path edit or another Push/Pull.
    ///
    /// The remote callback runs only after all local after-states validate.
    /// Local writes finish with `sync_config.json`, so a crash before that
    /// point can be repaired by rerunning with the same reviewed revision.
    pub fn clear_storage_data<T>(
        &self,
        storage_id: &StorageId,
        expected_config_revision: u64,
        clear_remote: impl FnOnce(&StorageConfig) -> Result<T, String>,
    ) -> Result<(T, StorageSyncMetadataResetResult), String> {
        let _operation = self.acquire_host_operation()?;
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let mut config = self.load_config_unlocked()?;
        if config.revision != expected_config_revision {
            return Err(format!(
                "storage configuration changed (expected revision {}, current {})",
                expected_config_revision, config.revision
            ));
        }
        let storage = config
            .storages
            .iter()
            .find(|storage| &storage.id == storage_id)
            .cloned()
            .ok_or_else(|| format!("unknown storage '{}'", storage_id))?;

        let mut materializations = self.load_materializations_unlocked(&config)?;
        let mut links_reset = 0_u64;
        let mut scope_bases_removed = 0_u64;
        let mut conversation_acceptances_removed = 0_u64;
        let mut config_changed = false;
        for link in config
            .links
            .iter_mut()
            .filter(|link| &link.storage_id == storage_id)
        {
            links_reset = links_reset.saturating_add(1);
            scope_bases_removed = scope_bases_removed.saturating_add(link.scope_bases.len() as u64);
            // A storage clear invalidates remote acceptance only. Keep the
            // local cursor and physical mapping that distinguish preserved
            // provider aliases from newly discovered local sessions.
            let acceptances_removed = link
                .conversation_checkpoints
                .values_mut()
                .flat_map(|threads| threads.values_mut())
                .filter_map(|checkpoint| {
                    checkpoint.accepted.take().map(|_| {
                        checkpoint.accepted_local_tail = None;
                        checkpoint.accepted_subagents.clear();
                        checkpoint.generation = checkpoint.generation.saturating_add(1);
                    })
                })
                .count() as u64;
            conversation_acceptances_removed =
                conversation_acceptances_removed.saturating_add(acceptances_removed);
            config_changed |= !link.scope_bases.is_empty() || acceptances_removed > 0;
            link.scope_bases.clear();
        }

        let materialization_count = materializations.records.len();
        materializations
            .records
            .retain(|record| &record.storage_id != storage_id);
        let materializations_removed =
            (materialization_count - materializations.records.len()) as u64;

        if config_changed {
            config.revision = config.revision.saturating_add(1);
        }
        if materializations_removed > 0 {
            materializations.revision = materializations.revision.saturating_add(1);
        }
        config.validate()?;
        materializations.validate(&config)?;

        let remote_result = clear_remote(&storage)?;

        if materializations_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.materializations_path(),
                &materializations,
                MAX_MATERIALIZATIONS_BYTES,
            )?;
        }
        if config_changed {
            write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)?;
        }

        Ok((
            remote_result,
            StorageSyncMetadataResetResult {
                storage_id: storage_id.clone(),
                config_revision: config.revision,
                links_reset,
                scope_bases_removed,
                conversation_acceptances_removed,
                materializations_removed,
            },
        ))
    }

    pub fn setup_draft_path(&self, draft_id: &SetupDraftId) -> PathBuf {
        self.root
            .join("project_drafts")
            .join(format!("{}.json", draft_id.as_str()))
    }

    pub fn setup_transaction_path(&self, draft_id: &SetupDraftId) -> PathBuf {
        self.root
            .join("setup_transactions")
            .join(format!("{}.json", draft_id.as_str()))
    }

    /// Load every setup draft.  A malformed draft file becomes a warning
    /// instead of hiding the readable drafts behind one error.
    pub fn list_setup_drafts(&self) -> Result<(Vec<ProjectSetupDraft>, Vec<String>), String> {
        let _guard = persistence_guard(&self.root)?;
        let (loaded, mut warnings): (Vec<ProjectSetupDraft>, _) =
            self.list_documents(&self.root.join("project_drafts"), MAX_SETUP_DRAFT_BYTES)?;
        let mut drafts = Vec::new();
        for draft in loaded {
            match draft.validate() {
                Ok(()) => drafts.push(draft),
                Err(error) => {
                    warnings.push(format!("invalid draft '{}': {}", draft.draft_id, error))
                }
            }
        }
        drafts.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.draft_id.cmp(&right.draft_id))
        });
        Ok((drafts, warnings))
    }

    pub fn load_setup_draft(
        &self,
        draft_id: &SetupDraftId,
    ) -> Result<Option<ProjectSetupDraft>, String> {
        let _guard = persistence_guard(&self.root)?;
        let draft: Option<ProjectSetupDraft> = read_json_bounded(
            &self.root,
            &self.setup_draft_path(draft_id),
            MAX_SETUP_DRAFT_BYTES,
        )?;
        if let Some(draft) = &draft {
            draft.validate()?;
            if &draft.draft_id != draft_id {
                return Err(format!(
                    "setup draft file '{}' contains draft '{}'",
                    draft_id, draft.draft_id
                ));
            }
        }
        Ok(draft)
    }

    /// Revision-guarded draft save.  Creation submits revision 0 against a
    /// missing file; updates must submit the revision they read.  The stored
    /// document gets the next revision, which is also returned.
    pub fn save_setup_draft(
        &self,
        mut draft: ProjectSetupDraft,
    ) -> Result<ProjectSetupDraft, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.setup_draft_path(&draft.draft_id);
        let current: Option<ProjectSetupDraft> =
            read_json_bounded(&self.root, &path, MAX_SETUP_DRAFT_BYTES)?;
        match &current {
            Some(existing) => {
                if draft.revision != existing.revision {
                    return Err(format!(
                        "setup draft changed (expected revision {}, current {})",
                        draft.revision, existing.revision
                    ));
                }
            }
            None => {
                if draft.revision != 0 {
                    return Err("new setup draft must start at revision 0".to_string());
                }
                let (existing, _) = self.list_documents::<ProjectSetupDraft>(
                    &self.root.join("project_drafts"),
                    MAX_SETUP_DRAFT_BYTES,
                )?;
                if existing.len() >= MAX_SETUP_DRAFTS {
                    return Err(format!(
                        "too many setup drafts (limit {}); discard one first",
                        MAX_SETUP_DRAFTS
                    ));
                }
            }
        }
        draft.revision = draft.revision.saturating_add(1);
        draft.validate()?;
        self.ensure_directory(path.parent().ok_or("setup draft has no parent")?)?;
        write_json_atomic(&self.root, &path, &draft, MAX_SETUP_DRAFT_BYTES)?;
        Ok(draft)
    }

    pub fn delete_setup_draft(&self, draft_id: &SetupDraftId) -> Result<bool, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.setup_draft_path(draft_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("remove setup draft '{}': {}", draft_id, error)),
        }
    }

    pub fn storage_setup_draft_path(&self, draft_id: &StorageSetupDraftId) -> PathBuf {
        self.root
            .join("storage_drafts")
            .join(format!("{}.json", draft_id.as_str()))
    }

    /// Load every storage setup draft.  A malformed draft file becomes a
    /// warning instead of hiding the readable drafts behind one error.
    /// Unlike the project-draft listing, warnings here are fixed sentences:
    /// serde's parse errors can embed string values from the file, and every
    /// storage draft file may hold a secret.
    pub fn list_storage_setup_drafts(
        &self,
    ) -> Result<(Vec<StorageSetupDraft>, Vec<String>), String> {
        let _guard = persistence_guard(&self.root)?;
        self.list_storage_setup_drafts_unlocked()
    }

    pub fn list_storage_setup_drafts_with_config(
        &self,
    ) -> Result<(SyncConfig, Vec<StorageSetupDraft>, Vec<String>), String> {
        let _guard = persistence_guard(&self.root)?;
        let config = self.load_config_unlocked()?;
        let (drafts, warnings) = self.list_storage_setup_drafts_unlocked()?;
        Ok((config, drafts, warnings))
    }

    fn list_storage_setup_drafts_unlocked(
        &self,
    ) -> Result<(Vec<StorageSetupDraft>, Vec<String>), String> {
        let directory = self.root.join("storage_drafts");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err(format!("'{}' is not a real directory", directory.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()))
            }
            Err(error) => return Err(format!("inspect '{}': {}", directory.display(), error)),
        }
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("list '{}': {}", directory.display(), error))?;
        let mut drafts = Vec::new();
        let mut warnings = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("list '{}': {}", directory.display(), error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let label = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            match read_json_bounded::<StorageSetupDraft>(
                &self.root,
                &path,
                MAX_STORAGE_SETUP_DRAFT_BYTES,
            ) {
                Ok(Some(draft)) => match draft.validate() {
                    Ok(()) => drafts.push(draft),
                    Err(_) => warnings.push(format!("invalid storage draft '{}'", label)),
                },
                Ok(None) => {}
                Err(_) => warnings.push(format!("unreadable storage draft '{}'", label)),
            }
        }
        drafts.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.draft_id.cmp(&right.draft_id))
        });
        Ok((drafts, warnings))
    }

    pub fn load_storage_setup_draft(
        &self,
        draft_id: &StorageSetupDraftId,
    ) -> Result<Option<StorageSetupDraft>, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_storage_setup_draft_unlocked(draft_id)
    }

    pub fn load_storage_setup_draft_with_config(
        &self,
        draft_id: &StorageSetupDraftId,
    ) -> Result<(SyncConfig, Option<StorageSetupDraft>), String> {
        let _guard = persistence_guard(&self.root)?;
        let config = self.load_config_unlocked()?;
        let draft = self.load_storage_setup_draft_unlocked(draft_id)?;
        Ok((config, draft))
    }

    fn load_storage_setup_draft_unlocked(
        &self,
        draft_id: &StorageSetupDraftId,
    ) -> Result<Option<StorageSetupDraft>, String> {
        // Read and parse failures collapse to one fixed sentence: the raw
        // error can quote file content, and this file holds a secret.
        let draft: Option<StorageSetupDraft> = read_json_bounded(
            &self.root,
            &self.storage_setup_draft_path(draft_id),
            MAX_STORAGE_SETUP_DRAFT_BYTES,
        )
        .map_err(|_| format!("storage draft '{}' is unreadable", draft_id))?;
        if let Some(draft) = &draft {
            draft
                .validate()
                .map_err(|_| format!("storage draft '{}' is unreadable", draft_id))?;
            if &draft.draft_id != draft_id {
                return Err(format!(
                    "storage draft file '{}' contains draft '{}'",
                    draft_id, draft.draft_id
                ));
            }
        }
        Ok(draft)
    }

    /// Revision-guarded storage draft save.  Creation submits revision 0
    /// against a missing file (and is bounded by the draft cap); updates must
    /// submit the revision they read.  The stored document gets the next
    /// revision, which is also returned.
    pub fn save_storage_setup_draft(
        &self,
        mut draft: StorageSetupDraft,
    ) -> Result<StorageSetupDraft, String> {
        // Keep every fallible step inside this closure so the draft is
        // returned intact only on success; its secrets zeroize on drop.
        let result = (|| -> Result<(), String> {
            let _guard = persistence_guard(&self.root)?;
            let path = self.storage_setup_draft_path(&draft.draft_id);
            draft.validate()?;
            let config = self.load_config_unlocked()?;
            // Secret values zeroize when the loaded draft drops.
            let current = self.load_storage_setup_draft_unlocked(&draft.draft_id)?;
            match current {
                Some(existing) => {
                    draft.validate_source_target(&config)?;
                    if draft.source != existing.source {
                        return Err("storage draft source changed".to_string());
                    }
                    if draft.revision != existing.revision {
                        return Err(format!(
                            "storage draft changed (expected revision {}, current {})",
                            draft.revision, existing.revision
                        ));
                    }
                }
                None => {
                    draft.validate_source(&config)?;
                    if draft.revision != 0 {
                        return Err("new storage draft must start at revision 0".to_string());
                    }
                    let existing = self.count_storage_setup_drafts_unlocked()?;
                    if existing >= MAX_STORAGE_SETUP_DRAFTS {
                        return Err(format!(
                            "too many storage drafts (limit {}); discard one first",
                            MAX_STORAGE_SETUP_DRAFTS
                        ));
                    }
                }
            }
            draft.revision = draft.revision.saturating_add(1);
            self.ensure_directory(path.parent().ok_or("storage draft has no parent")?)?;
            write_json_atomic(&self.root, &path, &draft, MAX_STORAGE_SETUP_DRAFT_BYTES)
        })();
        result?;
        Ok(draft)
    }

    /// Count every draft file, readable or not — an unreadable file still
    /// occupies its slot until discarded, so corruption cannot widen the cap.
    fn count_storage_setup_drafts_unlocked(&self) -> Result<usize, String> {
        let directory = self.root.join("storage_drafts");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err(format!("'{}' is not a real directory", directory.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(format!("inspect '{}': {}", directory.display(), error)),
        }
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("list '{}': {}", directory.display(), error))?;
        let mut count = 0;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("list '{}': {}", directory.display(), error))?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn delete_storage_setup_draft(
        &self,
        draft_id: &StorageSetupDraftId,
    ) -> Result<bool, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.storage_setup_draft_path(draft_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("remove storage draft '{}': {}", draft_id, error)),
        }
    }

    pub fn list_setup_transactions(&self) -> Result<(Vec<SetupTransaction>, Vec<String>), String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let directory = self.root.join("setup_transactions");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err("setup transaction storage is not a real directory".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()))
            }
            Err(error) => return Err(format!("inspect setup transaction storage: {}", error)),
        }
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("list setup transaction storage: {}", error))?;
        let mut loaded = Vec::new();
        let mut warnings = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("list setup transaction: {}", error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let label = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown setup transaction")
                .to_string();
            match self.read_setup_transaction_unlocked(&path) {
                Ok(Some(transaction)) => loaded.push(transaction),
                Ok(None) => {}
                Err(error) => warnings.push(format!(
                    "unreadable '{}': {}",
                    label,
                    error.replace(self.root.to_string_lossy().as_ref(), "<local state>")
                )),
            }
        }
        let mut transactions = Vec::new();
        for transaction in loaded {
            match transaction.validate() {
                // An invalid transaction is preserved on disk for inspection;
                // recovery must not complete or delete what it cannot trust.
                Ok(()) => transactions.push(transaction),
                Err(error) => warnings.push(format!(
                    "invalid setup transaction '{}': {}",
                    transaction.draft_id, error
                )),
            }
        }
        Ok((transactions, warnings))
    }

    pub fn load_setup_transaction(
        &self,
        draft_id: &SetupDraftId,
    ) -> Result<Option<SetupTransaction>, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let transaction =
            self.read_setup_transaction_unlocked(&self.setup_transaction_path(draft_id))?;
        if let Some(transaction) = &transaction {
            transaction.validate()?;
            if &transaction.draft_id != draft_id {
                return Err(format!(
                    "setup transaction file '{}' contains draft '{}'",
                    draft_id, transaction.draft_id
                ));
            }
        }
        Ok(transaction)
    }

    fn read_setup_transaction_unlocked(
        &self,
        path: &Path,
    ) -> Result<Option<SetupTransaction>, String> {
        let Some(value) =
            read_json_bounded::<Value>(&self.root, path, MAX_SETUP_TRANSACTION_BYTES)?
        else {
            return Ok(None);
        };
        Ok(Some(decode_setup_transaction(value)?))
    }

    /// A transaction is one finalize attempt's committed intent; replacing an
    /// existing file would silently reinterpret an in-flight application.
    pub fn save_setup_transaction(&self, transaction: &SetupTransaction) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        transaction.validate()?;
        let path = self.setup_transaction_path(&transaction.draft_id);
        self.ensure_directory(path.parent().ok_or("setup transaction has no parent")?)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!(
                "setup transaction for draft '{}' already exists",
                transaction.draft_id
            ));
        }
        write_json_atomic(&self.root, &path, transaction, MAX_SETUP_TRANSACTION_BYTES)
    }

    pub fn delete_setup_transaction(&self, draft_id: &SetupDraftId) -> Result<bool, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.setup_transaction_path(draft_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!(
                "remove setup transaction '{}': {}",
                draft_id, error
            )),
        }
    }

    fn list_documents<T: DeserializeOwned>(
        &self,
        directory: &Path,
        max_bytes: u64,
    ) -> Result<(Vec<T>, Vec<String>), String> {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err(format!("'{}' is not a real directory", directory.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()))
            }
            Err(error) => return Err(format!("inspect '{}': {}", directory.display(), error)),
        }
        let entries = fs::read_dir(directory)
            .map_err(|error| format!("list '{}': {}", directory.display(), error))?;
        let mut documents = Vec::new();
        let mut warnings = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("list '{}': {}", directory.display(), error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            match read_json_bounded::<T>(&self.root, &path, max_bytes) {
                Ok(Some(document)) => documents.push(document),
                Ok(None) => {}
                Err(error) => {
                    let label = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("unknown document");
                    warnings.push(format!(
                        "unreadable '{}': {}",
                        label,
                        error.replace(self.root.to_string_lossy().as_ref(), "<local state>")
                    ));
                }
            }
        }
        Ok((documents, warnings))
    }

    fn load_config_unlocked(&self) -> Result<SyncConfig, String> {
        self.recover_project_bundle_unlocked()?;
        let config: SyncConfig =
            read_json_bounded(&self.root, &self.config_path(), MAX_CONFIG_BYTES)?
                .unwrap_or_default();
        config.validate()?;
        Ok(config)
    }

    fn load_or_create_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        self.recover_project_bundle_unlocked()?;
        self.recover_host_rotation_unlocked()?;
        match self.load_host_identity_document_classified_unlocked() {
            Ok(Some(identity)) => Ok(identity),
            Ok(None) => self.auto_register_host_identity_unlocked(),
            Err(BoundedJsonReadError::Io(error)) => Err(error),
            // A damaged identity document cannot be repaired. Preserve its
            // bytes and register a fresh host in place, without asking.
            Err(BoundedJsonReadError::Corrupt(_)) => {
                self.quarantine_file_unlocked(&self.host_identity_path(), "host-identity")?;
                self.auto_register_host_identity_unlocked()
            }
        }
    }

    /// Establish a usable identity without user interaction. A clean tree
    /// gets a plain first-run identity; any surviving host-scoped documents
    /// are rebased onto the fresh identity as detached records.
    fn auto_register_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        let mut has_host_scoped_state = false;
        for path in [
            self.machine_projects_path(),
            self.conversation_bindings_path(),
            self.materializations_path(),
        ] {
            if self.path_exists_unlocked(&path)? {
                has_host_scoped_state = true;
                break;
            }
        }
        if !has_host_scoped_state {
            return self.create_host_identity_unlocked();
        }
        self.replace_host_identity_unlocked()
    }

    #[cfg(test)]
    fn load_host_identity_document_unlocked(&self) -> Result<Option<HostIdentity>, String> {
        self.load_host_identity_document_classified_unlocked()
            .map_err(BoundedJsonReadError::into_message)
    }

    fn load_host_identity_document_classified_unlocked(
        &self,
    ) -> Result<Option<HostIdentity>, BoundedJsonReadError> {
        let identity: Option<HostIdentity> = read_json_bounded_classified(
            &self.root,
            &self.host_identity_path(),
            MAX_HOST_IDENTITY_BYTES,
        )?;
        if let Some(identity) = &identity {
            identity.validate().map_err(BoundedJsonReadError::Corrupt)?;
        }
        Ok(identity)
    }

    fn create_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        let identity = self.prepare_host_identity_unlocked()?;
        write_json_atomic(
            &self.root,
            &self.host_identity_path(),
            &identity,
            MAX_HOST_IDENTITY_BYTES,
        )?;
        Ok(identity)
    }

    fn prepare_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        let timestamp = now_secs()?;
        let identity = HostIdentity {
            schema: HOST_IDENTITY_SCHEMA,
            revision: 0,
            host_id: HostId::generate()?,
            local_display_name: None,
            created_at: timestamp,
            updated_at: timestamp,
        };
        identity.validate()?;
        Ok(identity)
    }

    fn load_host_rotation_transaction_classified_unlocked(
        &self,
    ) -> Result<Option<HostRotationTransaction>, BoundedJsonReadError> {
        read_json_bounded_classified(
            &self.root,
            &self.host_rotation_path(),
            MAX_HOST_ROTATION_BYTES,
        )
    }

    fn path_exists_unlocked(&self, path: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("inspect host-scoped local state: {}", error)),
        }
    }

    fn validate_setup_transactions_unlocked(&self) -> Result<(), String> {
        let directory = self.root.join("setup_transactions");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err("setup transaction storage is not a real directory".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("inspect setup transaction storage: {}", error)),
        }
        let mut count = 0_usize;
        let mut quarantines = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("list setup transaction storage: {}", error))?
        {
            let path = entry
                .map_err(|error| format!("list setup transaction: {}", error))?
                .path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            count = count.saturating_add(1);
            if count > MAX_SETUP_DRAFTS {
                return Err("too many setup transactions to rotate safely".to_string());
            }
            let decoded =
                read_json_bounded::<Value>(&self.root, &path, MAX_SETUP_TRANSACTION_BYTES)
                    .and_then(|value| {
                        value.ok_or_else(|| {
                            "setup transaction disappeared during rotation".to_string()
                        })
                    })
                    .and_then(decode_setup_transaction);
            match decoded {
                Ok(_) => {}
                Err(_) => quarantines.push(path),
            }
        }
        for path in quarantines {
            self.quarantine_file_unlocked(&path, "setup-transaction")?;
        }
        Ok(())
    }

    /// Move an untrusted recovery artifact out of its live location without
    /// deleting it. Quarantined files deliberately do not retain a `.json`
    /// suffix, so ordinary journal scans cannot accidentally reactivate them.
    fn quarantine_file_unlocked(&self, path: &Path, category: &str) -> Result<(), String> {
        self.quarantine_file_with_sync_unlocked(path, category, sync_directory)
    }

    fn quarantine_file_with_sync_unlocked(
        &self,
        path: &Path,
        category: &str,
        mut sync: impl FnMut(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        ensure_no_symlinks(&self.root, path)?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A prior rename may have completed even though its directory
                // sync reported an error. Make the quarantine directory entry
                // durable first, then its contents, and only then the source
                // directory deletion before treating the missing path as
                // settled.
                let directory = self.root.join("host_recovery_quarantine");
                if self.path_exists_unlocked(&directory)? {
                    ensure_real_directory(&directory)?;
                    sync(&directory)?;
                }
                sync(&self.root)?;
                if let Some(parent) = path.parent().filter(|parent| *parent != self.root) {
                    sync(parent)?;
                }
                return Ok(());
            }
            Err(error) => return Err(format!("inspect {} recovery artifact: {}", category, error)),
        };
        if !metadata.file_type().is_file() {
            return Err(format!(
                "{} recovery artifact is not a regular file",
                category
            ));
        }
        let directory = self.root.join("host_recovery_quarantine");
        self.ensure_directory(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("secure host recovery quarantine: {}", error))?;
        }
        // The quarantine directory itself must be durable before its first
        // preserved entry can replace a source from another directory.
        sync(&self.root)?;
        let destination = directory.join(format!(
            "{}-{}.quarantined",
            category,
            generated_named_id("artifact")?
        ));
        ensure_no_symlinks(&self.root, &destination)?;
        // Generic quarantine has no durable journal from which to reconstruct
        // a random destination. Add and persist a second link before removing
        // the live link, so any failure before destination durability leaves
        // the source discoverable for a whole-operation retry.
        fs::hard_link(path, &destination)
            .map_err(|error| format!("preserve {} recovery artifact: {}", category, error))?;
        sync(&directory)?;
        fs::remove_file(path)
            .map_err(|error| format!("retire live {} recovery artifact: {}", category, error))?;
        if let Some(parent) = path.parent() {
            sync(parent)?;
        }
        Ok(())
    }

    fn host_rotation_quarantine_destination(
        &self,
        transaction: &HostRotationTransaction,
        category: &str,
    ) -> PathBuf {
        self.root.join("host_recovery_quarantine").join(format!(
            "{}-{}.quarantined",
            category,
            transaction.next_identity.host_id.as_str()
        ))
    }

    fn recovery_artifacts_share_identity_unlocked(
        &self,
        source: &Path,
        destination: &Path,
        category: &str,
    ) -> Result<bool, String> {
        ensure_no_symlinks(&self.root, source)?;
        ensure_no_symlinks(&self.root, destination)?;
        let source_metadata = fs::symlink_metadata(source)
            .map_err(|error| format!("inspect live {} recovery artifact: {}", category, error))?;
        let destination_metadata = fs::symlink_metadata(destination).map_err(|error| {
            format!(
                "inspect preserved {} recovery artifact: {}",
                category, error
            )
        })?;
        if !source_metadata.file_type().is_file()
            || source_metadata.file_type().is_symlink()
            || !destination_metadata.file_type().is_file()
            || destination_metadata.file_type().is_symlink()
        {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(source_metadata.dev() == destination_metadata.dev()
                && source_metadata.ino() == destination_metadata.ino())
        }
        #[cfg(windows)]
        {
            let source_file = fs::File::open(source)
                .map_err(|error| format!("open live {} recovery artifact: {}", category, error))?;
            let destination_file = fs::File::open(destination).map_err(|error| {
                format!("open preserved {} recovery artifact: {}", category, error)
            })?;
            let source_identity = windows_file_identity(
                &source_file,
                &format!("live {} recovery artifact", category),
            )?;
            let destination_identity = windows_file_identity(
                &destination_file,
                &format!("preserved {} recovery artifact", category),
            )?;
            Ok(source_identity.0 == destination_identity.0
                && source_identity.1 == destination_identity.1)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (source_metadata, destination_metadata);
            Err(format!(
                "cannot safely compare {} recovery artifacts on this platform",
                category
            ))
        }
    }

    /// Rotation quarantine destinations are derived from the durable journal,
    /// so recovery can prove that a missing source was preserved and can retry
    /// either directory sync after a rename reported uncertainty.
    fn quarantine_host_rotation_file_unlocked(
        &self,
        transaction: &HostRotationTransaction,
        path: &Path,
        category: &str,
    ) -> Result<(), String> {
        ensure_no_symlinks(&self.root, path)?;
        let directory = self.root.join("host_recovery_quarantine");
        let destination = self.host_rotation_quarantine_destination(transaction, category);
        let source_metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("inspect {} recovery artifact: {}", category, error)),
        };
        if source_metadata.is_none() {
            ensure_no_symlinks(&self.root, &destination)?;
            let destination_metadata = fs::symlink_metadata(&destination).map_err(|error| {
                format!(
                    "confirm preserved {} recovery artifact: {}",
                    category, error
                )
            })?;
            if !destination_metadata.file_type().is_file()
                || destination_metadata.file_type().is_symlink()
            {
                return Err(format!(
                    "preserved {} recovery artifact is not a regular file",
                    category
                ));
            }
            ensure_real_directory(&directory)?;
            sync_directory(&directory)?;
            sync_directory(&self.root)?;
            if let Some(parent) = path.parent().filter(|parent| *parent != self.root) {
                sync_directory(parent)?;
            }
            return Ok(());
        }
        if !source_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.file_type().is_file())
        {
            return Err(format!(
                "{} recovery artifact is not a regular file",
                category
            ));
        }
        self.ensure_directory(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("secure host recovery quarantine: {}", error))?;
        }
        // Persist the quarantine directory itself before moving the source.
        sync_directory(&self.root)?;
        ensure_no_symlinks(&self.root, &destination)?;
        match fs::symlink_metadata(&destination) {
            Ok(_)
                if self.recovery_artifacts_share_identity_unlocked(
                    path,
                    &destination,
                    category,
                )? =>
            {
                // A prior preserve-before-delete attempt created the durable
                // link but returned before retiring the live path. Resume it.
            }
            Ok(_) => {
                return Err(format!(
                    "preserved {} recovery artifact differs from the live source",
                    category
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::hard_link(path, &destination).map_err(|error| {
                    format!("preserve {} recovery artifact: {}", category, error)
                })?;
                if !self.recovery_artifacts_share_identity_unlocked(path, &destination, category)? {
                    return Err(format!(
                        "preserved {} recovery artifact does not match the live source",
                        category
                    ));
                }
            }
            Err(error) => {
                return Err(format!(
                    "inspect preserved {} recovery artifact: {}",
                    category, error
                ))
            }
        }
        // Preserve the destination before making the source deletion durable.
        sync_directory(&directory)?;
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "retire live {} recovery artifact: {}",
                    category, error
                ))
            }
        }
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }

    fn recover_host_rotation_unlocked(&self) -> Result<(), String> {
        let transaction = match self.load_host_rotation_transaction_classified_unlocked() {
            Ok(None) => return Ok(()),
            Ok(Some(transaction)) => transaction,
            Err(BoundedJsonReadError::Io(error)) => return Err(error),
            // An unreadable journal cannot be replayed. Preserve its bytes;
            // identity loading then continues from the live documents.
            Err(BoundedJsonReadError::Corrupt(_)) => {
                return self.quarantine_file_unlocked(&self.host_rotation_path(), "host-rotation")
            }
        };
        // Keep durability failures distinct from parse/validation corruption:
        // only the latter may be quarantined.
        sync_directory(&self.root)?;
        let config = self.load_config_unlocked()?;
        if self
            .validate_host_rotation_payload_unlocked(&transaction, &config)
            .is_err()
        {
            return self.quarantine_file_unlocked(&self.host_rotation_path(), "host-rotation");
        }
        // Replaying a validated journal is idempotent: every document write
        // is an atomic replacement and recorded quarantine intent recognizes
        // already-written replacements. The journal retires only afterwards.
        self.commit_host_rotation_unlocked(&transaction)
    }

    fn validate_host_rotation_payload_unlocked(
        &self,
        transaction: &HostRotationTransaction,
        config: &SyncConfig,
    ) -> Result<(), String> {
        if transaction.schema != HOST_ROTATION_SCHEMA {
            return Err("invalid host rotation transaction".to_string());
        }
        transaction.next_identity.validate()?;
        transaction.machine_projects.validate(config)?;
        if transaction.machine_projects.bindings.iter().any(|binding| {
            binding.host_id != transaction.next_identity.host_id
                || binding.state != mallard_domain::BindingState::Detached
        }) {
            return Err("host rotation contains a live or mismatched project binding".to_string());
        }
        transaction.conversation_bindings.validate(
            &transaction.next_identity,
            &transaction.machine_projects,
            config,
            &tail_layout_codec_for,
        )?;
        if transaction
            .conversation_bindings
            .bindings
            .iter()
            .any(|binding| {
                binding.materialization_state
                    == mallard_domain::ConversationMaterializationState::Resumable
            })
        {
            return Err(
                "host rotation must not pre-authorize resumable conversation bindings".to_string(),
            );
        }
        transaction.materializations.validate(config)?;
        if transaction
            .materializations
            .records
            .iter()
            .any(|record| record.status != mallard_domain::MaterializationStatus::Detached)
        {
            return Err("host rotation contains a live materialization record".to_string());
        }
        Ok(())
    }

    fn quarantine_unless_replacement_unlocked<T>(
        &self,
        transaction: &HostRotationTransaction,
        enabled: bool,
        path: &Path,
        category: &str,
        max_bytes: u64,
        replacement: &T,
    ) -> Result<(), String>
    where
        T: DeserializeOwned + PartialEq,
    {
        if !enabled {
            return Ok(());
        }
        if read_json_bounded::<T>(&self.root, path, max_bytes)
            .ok()
            .flatten()
            .as_ref()
            .is_some_and(|current| current == replacement)
        {
            return Ok(());
        }
        self.quarantine_host_rotation_file_unlocked(transaction, path, category)
    }

    fn apply_host_rotation_quarantine_unlocked(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<(), String> {
        self.quarantine_unless_replacement_unlocked(
            transaction,
            transaction.quarantine.machine_projects,
            &self.machine_projects_path(),
            "machine-projects",
            MAX_BINDINGS_BYTES,
            &transaction.machine_projects,
        )?;
        self.quarantine_unless_replacement_unlocked(
            transaction,
            transaction.quarantine.conversation_bindings,
            &self.conversation_bindings_path(),
            "conversation-bindings",
            MAX_CONVERSATION_BINDINGS_BYTES,
            &transaction.conversation_bindings,
        )?;
        self.quarantine_unless_replacement_unlocked(
            transaction,
            transaction.quarantine.materializations,
            &self.materializations_path(),
            "materializations",
            MAX_MATERIALIZATIONS_BYTES,
            &transaction.materializations,
        )?;
        Ok(())
    }

    fn commit_host_rotation_unlocked(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<(), String> {
        // Quarantine intent is part of the durable transaction. Replaying it
        // before any replacement write preserves damaged originals across a
        // crash. Already-written replacement documents are recognized and
        // left in place, making recovery idempotent.
        self.apply_host_rotation_quarantine_unlocked(transaction)?;
        write_json_atomic(
            &self.root,
            &self.machine_projects_path(),
            &transaction.machine_projects,
            MAX_BINDINGS_BYTES,
        )?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            &transaction.conversation_bindings,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )?;
        write_json_atomic(
            &self.root,
            &self.materializations_path(),
            &transaction.materializations,
            MAX_MATERIALIZATIONS_BYTES,
        )?;
        write_json_atomic(
            &self.root,
            &self.host_identity_path(),
            &transaction.next_identity,
            MAX_HOST_IDENTITY_BYTES,
        )?;
        self.finish_host_rotation_cleanup_unlocked()
    }

    fn finish_host_rotation_cleanup_unlocked(&self) -> Result<(), String> {
        // Replacement documents become durable before the journal retires.
        sync_directory(&self.root)?;
        let path = self.host_rotation_path();
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => sync_directory(&self.root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                sync_directory(&self.root)
            }
            Err(error) => Err(format!("retire host rotation transaction: {}", error)),
        }
    }

    fn load_bindings_unlocked(
        &self,
        config: &SyncConfig,
        identity: &HostIdentity,
    ) -> Result<MachineProjectState, String> {
        let bindings = self.load_bindings_document_unlocked(config)?;
        if bindings
            .bindings
            .iter()
            .any(|binding| binding.host_id != identity.host_id)
        {
            return Err(
                "project bindings belong to a different host; register as a new host".to_string(),
            );
        }
        Ok(bindings)
    }

    fn load_bindings_document_unlocked(
        &self,
        config: &SyncConfig,
    ) -> Result<MachineProjectState, String> {
        let path = self.machine_projects_path();
        let Some(value) = read_json_bounded::<Value>(&self.root, &path, MAX_BINDINGS_BYTES)? else {
            return Ok(MachineProjectState::default());
        };
        let bindings: MachineProjectState = serde_json::from_value(value)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        bindings
            .validate(config)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        Ok(bindings)
    }

    fn load_materializations_unlocked(
        &self,
        config: &SyncConfig,
    ) -> Result<Materializations, String> {
        let path = self.materializations_path();
        let Some(value) =
            read_json_bounded::<Value>(&self.root, &path, MAX_MATERIALIZATIONS_BYTES)?
        else {
            return Ok(Materializations::default());
        };
        let materializations: Materializations = serde_json::from_value(value)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        materializations.validate(config)?;
        Ok(materializations)
    }

    fn read_conversation_bindings_unlocked(&self) -> Result<Option<ConversationBindings>, String> {
        read_json_bounded(
            &self.root,
            &self.conversation_bindings_path(),
            MAX_CONVERSATION_BINDINGS_BYTES,
        )
    }

    fn load_conversation_bindings_document_unlocked(
        &self,
        config: &SyncConfig,
        machine: &MachineProjectState,
        identity: &HostIdentity,
    ) -> Result<ConversationBindings, String> {
        let bindings = self
            .read_conversation_bindings_unlocked()?
            .unwrap_or_else(|| ConversationBindings::empty(identity.host_id.clone()));
        bindings.validate(identity, machine, config, &tail_layout_codec_for)?;
        Ok(bindings)
    }

    fn ensure_root(&self) -> Result<(), String> {
        let parent = self
            .root
            .parent()
            .ok_or_else(|| format!("metadata root '{}' has no parent", self.root.display()))?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create metadata parent directory '{}': {}",
                parent.display(),
                error
            )
        })?;
        match fs::symlink_metadata(&self.root) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(format!(
                    "metadata directory '{}' is not a real directory",
                    self.root.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_metadata_root(&self.root)?;
            }
            Err(error) => {
                return Err(format!(
                    "inspect metadata directory '{}': {}",
                    self.root.display(),
                    error
                ));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    format!(
                        "secure metadata directory '{}': {error}",
                        self.root.display()
                    )
                },
            )?;
        }
        ensure_no_symlinks(&self.root, &self.root).map(|_| ())
    }

    pub(crate) fn ensure_directory(&self, directory: &Path) -> Result<(), String> {
        let relative = directory.strip_prefix(&self.root).map_err(|_| {
            format!(
                "Mallard state path '{}' escapes '{}'",
                directory.display(),
                self.root.display()
            )
        })?;
        let mut current = self.root.clone();
        ensure_real_directory(&current)?;
        for component in relative.components() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(format!(
                        "Mallard state directory '{}' is not a real directory",
                        current.display()
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {
                            let parent = current.parent().ok_or_else(|| {
                                format!(
                                    "Mallard state directory '{}' has no parent",
                                    current.display()
                                )
                            })?;
                            sync_directory(parent)?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            ensure_real_directory(&current)?;
                            let parent = current.parent().ok_or_else(|| {
                                format!(
                                    "Mallard state directory '{}' has no parent",
                                    current.display()
                                )
                            })?;
                            sync_directory(parent)?;
                        }
                        Err(error) => {
                            return Err(format!(
                                "create Mallard state directory '{}': {}",
                                current.display(),
                                error
                            ));
                        }
                    }
                }
                Err(error) => {
                    return Err(format!("inspect '{}': {}", current.display(), error));
                }
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File, description: &str) -> Result<(u32, u64, u32), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let success =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if success == 0 {
        return Err(format!(
            "inspect {} file identity: {}",
            description,
            std::io::Error::last_os_error()
        ));
    }
    let file_index = ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64;
    Ok((
        information.dwVolumeSerialNumber,
        file_index,
        information.nNumberOfLinks,
    ))
}

fn decode_setup_transaction(value: Value) -> Result<SetupTransaction, String> {
    let transaction: SetupTransaction = serde_json::from_value(value)
        .map_err(|error| format!("parse setup transaction: {}", error))?;
    transaction.validate()?;
    Ok(transaction)
}

fn now_secs() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock is before the Unix epoch: {}", error))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    let directory = fs::File::open(path)
        .map_err(|error| format!("open directory '{}' for sync: {}", path.display(), error))?;
    directory
        .sync_all()
        .map_err(|error| format!("sync directory '{}': {}", path.display(), error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Count files and bytes under a directory without following symlinks — a
/// link contributes its own length, never its target's. Entries that vanish
/// mid-walk are skipped: the walk runs under a live app.
fn walk_local_state(dir: &Path) -> Result<(u64, u64), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(error) => return Err(format!("read directory '{}': {}", dir.display(), error)),
    };
    let mut files = 0u64;
    let mut bytes = 0u64;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read directory '{}': {}", dir.display(), error)),
        };
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("inspect '{}': {}", entry.path().display(), error));
            }
        };
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            let (nested_files, nested_bytes) = walk_local_state(&entry.path())?;
            files += nested_files;
            bytes += nested_bytes;
        } else {
            files += 1;
            bytes += metadata.len();
        }
    }
    Ok((files, bytes))
}

pub(crate) fn persistence_guard(root: &Path) -> Result<PersistenceGuard, String> {
    let process = PERSISTENCE_LOCK
        .lock()
        .map_err(|_| "project-sync persistence lock is poisoned".to_string())?;
    let path = root.join("persistence.lock");
    ensure_no_symlinks(root, &path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("open project-sync persistence lock: {}", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect project-sync persistence lock: {}", error))?;
    if !metadata.file_type().is_file() {
        return Err("project-sync persistence lock is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 {
            return Err("project-sync persistence lock has unsafe hard links".to_string());
        }
        if metadata.permissions().mode() & 0o7777 != 0o600 {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("secure project-sync persistence lock: {}", error))?;
        }
    }
    file.lock()
        .map_err(|error| format!("lock project-sync persistence: {}", error))?;
    Ok(PersistenceGuard {
        _process: process,
        _file: file,
    })
}

fn create_metadata_root(path: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        // Multiple startup commands can all observe a missing first-run root.
        // The process that loses creation must validate the winner rather than
        // report a harmless EEXIST as a backend failure.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure_real_directory(path)
        }
        Err(error) => Err(format!(
            "create metadata directory '{}': {}",
            path.display(),
            error
        )),
    }
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect directory '{}': {}", path.display(), error))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("'{}' is not a real directory", path.display()));
    }
    Ok(())
}

fn ensure_no_symlinks(root: &Path, destination: &Path) -> Result<Option<fs::Metadata>, String> {
    let relative = destination.strip_prefix(root).map_err(|_| {
        format!(
            "Mallard state path '{}' escapes '{}'",
            destination.display(),
            root.display()
        )
    })?;
    let mut current = root.to_path_buf();
    let mut leaf_metadata = None;
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(format!(
                        "Mallard state path traverses symlink '{}'",
                        current.display()
                    ));
                }
                leaf_metadata = Some(metadata);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                leaf_metadata = None;
            }
            Err(error) => return Err(format!("inspect '{}': {}", current.display(), error)),
        }
    }
    Ok(leaf_metadata)
}

pub(crate) fn read_json_bounded<T: DeserializeOwned>(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<Option<T>, String> {
    read_json_bounded_classified(root, path, max_bytes).map_err(BoundedJsonReadError::into_message)
}

fn read_json_bounded_classified<T: DeserializeOwned>(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<Option<T>, BoundedJsonReadError> {
    #[cfg(test)]
    tests::JSON_READS.with(|count| count.set(count.get() + 1));
    let Some(metadata) = ensure_no_symlinks(root, path).map_err(BoundedJsonReadError::Io)? else {
        return Ok(None);
    };
    if !metadata.file_type().is_file() {
        return Err(BoundedJsonReadError::Io(format!(
            "'{}' is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > max_bytes {
        return Err(BoundedJsonReadError::Corrupt(format!(
            "'{}' exceeds the {} byte limit",
            path.display(),
            max_bytes
        )));
    }
    let file = fs::File::open(path).map_err(|error| {
        BoundedJsonReadError::Io(format!("open '{}': {}", path.display(), error))
    })?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            BoundedJsonReadError::Io(format!("read '{}': {}", path.display(), error))
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(BoundedJsonReadError::Io(format!(
            "'{}' changed beyond the {} byte limit while reading",
            path.display(),
            max_bytes
        )));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        BoundedJsonReadError::Corrupt(format!("parse '{}': {}", path.display(), error))
    })
}

fn read_path_origin_bytes(
    root: &Path,
    path: &Path,
    reference: &mallard_domain::PathOriginsRef,
) -> Result<Vec<u8>, String> {
    reference.validate()?;
    let metadata = ensure_no_symlinks(root, path)?.ok_or("path-origin file is missing")?;
    if !metadata.is_file() || metadata.len() != reference.size_bytes {
        return Err("path-origin file has the wrong type or size".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("read path origins: {error}"))?;
    if !file
        .metadata()
        .map_err(|_| "inspect path origins")?
        .is_file()
    {
        return Err("path origins are not a regular file".into());
    }
    let mut bytes = Vec::with_capacity(reference.size_bytes as usize);
    file.take(reference.size_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read path origins: {error}"))?;
    if bytes.len() as u64 != reference.size_bytes
        || mallard_domain::sha256_hex(&bytes) != reference.sha256
    {
        return Err("path-origin file failed its integrity check".into());
    }
    Ok(bytes)
}

pub fn write_json_atomic<T: Serialize>(
    root: &Path,
    path: &Path,
    value: &T,
    max_bytes: u64,
) -> Result<(), String> {
    #[cfg(test)]
    tests::JSON_WRITES.with(|count| count.set(count.get() + 1));
    #[cfg(any(test, feature = "test-support"))]
    let fault = test_support::take_json_write_fault(path);
    #[cfg(any(test, feature = "test-support"))]
    if fault == Some(false) {
        return Err("injected state write failure before replacement".into());
    }
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    write_bytes_atomic(root, path, &bytes, max_bytes)?;
    #[cfg(any(test, feature = "test-support"))]
    if fault == Some(true) {
        return Err("injected state write failure after replacement".into());
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    thread_local! {
        static JSON_WRITE_FAULT: RefCell<Option<(PathBuf, bool)>> = const { RefCell::new(None) };
    }

    /// Fail one write to an exact fixture path on the calling test thread.
    pub fn fail_next_json_write(path: &Path, after_replacement: bool) {
        JSON_WRITE_FAULT.with(|fault| {
            assert!(fault
                .borrow_mut()
                .replace((path.to_path_buf(), after_replacement))
                .is_none());
        });
    }

    pub(super) fn take_json_write_fault(path: &Path) -> Option<bool> {
        JSON_WRITE_FAULT.with(|fault| {
            let mut fault = fault.borrow_mut();
            if fault.as_ref().is_some_and(|(target, _)| target == path) {
                fault.take().map(|(_, after)| after)
            } else {
                None
            }
        })
    }
}

fn write_json_atomic_compact<T: Serialize>(
    root: &Path,
    path: &Path,
    value: &T,
    max_bytes: u64,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    write_bytes_atomic(root, path, &bytes, max_bytes)
}

pub(crate) fn write_bytes_atomic(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    max_bytes: u64,
) -> Result<(), String> {
    ensure_no_symlinks(root, path)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("'{}' has no parent", path.display()))?;
    ensure_real_directory(parent)?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "serialized '{}' exceeds the {} byte limit",
            path.display(),
            max_bytes
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create temporary file in '{}': {}", parent.display(), error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure temporary '{}': {}", path.display(), error))?;
    }
    temporary
        .as_file_mut()
        .write_all(bytes)
        .map_err(|error| format!("write temporary '{}': {}", path.display(), error))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|error| format!("sync temporary '{}': {}", path.display(), error))?;
    temporary
        .persist(path)
        .map_err(|error| format!("publish '{}': {}", path.display(), error.error))?;
    sync_directory(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_providers::local::local_storage_config;
    use mallard_domain::{
        generated_named_id, AcceptedConversationVersion, BindingState, BundleId, BundleRecipe,
        CanonicalConversationMessage, ConversationCheckpoint, ConversationMaterializationState,
        LocalConversationBinding, LocalProjectRegistration, LocalProviderProfileId,
        MaterializationId, MaterializationRecord, MaterializationStatus, MessageId,
        PhysicalMappingState, PlanId, PreservedProvenance, ProjectBinding, ProjectStorageLink,
        ProviderKey, ProviderProfile, RepositoryBase, RootTailLayout, ScopeBase, StorageConfig,
        StorageId, SyncScopeKey, TailPartitionKind, ThreadId, LOCAL_SCHEMA, PORTABLE_TAIL_CODEC_V1,
    };
    use std::collections::BTreeMap;

    thread_local! {
        pub(super) static JSON_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static JSON_WRITES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn codex_key() -> ProviderKey {
        ProviderKey::parse("codex").unwrap()
    }

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path()).unwrap()
    }

    fn pull_after_state(state: &LockedConversationPull<'_>) -> (SyncConfig, ConversationBindings) {
        let current = state.current().unwrap();
        let mut config = current.config.clone();
        let mut bindings = current.conversation_bindings.clone();
        config.revision = config.revision.saturating_add(1);
        bindings.revision = bindings.revision.saturating_add(1);
        (config, bindings)
    }

    fn push_after_state(repo: &LocalStateRepository) -> ConversationPublicationState {
        let current = repo.load_project_activity_state_snapshot().unwrap();
        let mut config = current.config.clone();
        let mut bindings = current.conversation_bindings.clone();
        config.revision = config.revision.saturating_add(1);
        bindings.revision = bindings.revision.saturating_add(1);
        ConversationPublicationState {
            sync_config_before: current.config,
            sync_config_after: config,
            bindings_before: current.conversation_bindings,
            bindings_after: bindings,
        }
    }

    #[test]
    fn push_batch_writes_each_document_once() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _host = repo.acquire_host_operation().unwrap();
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(&repo, &temp, &identity);
        let mut state = push_after_state(&repo);
        state.sync_config_after.projects[0].display_name = "Published batch".into();
        let before_revision = repo.load_materializations().unwrap().revision;
        let before_writes = JSON_WRITES.with(std::cell::Cell::get);
        repo.commit_conversation_publication(&state, |materializations| {
            for index in 0..3 {
                materializations.records.push(MaterializationRecord {
                    materialization_id: MaterializationId::parse(format!(
                        "materialization-{index}"
                    ))?,
                    plan_id: PlanId::parse(format!("plan-{index}"))?,
                    host_id: identity.host_id.clone(),
                    replica_id: replica_id.clone(),
                    local_project_id: project().local_project_id,
                    storage_id: StorageId::parse("storage-push")?,
                    bundle_id: project().bundle_id,
                    revision: 1,
                    commit_id: "commit-push".into(),
                    manifest_sha256: "a".repeat(64),
                    binding_revision: 0,
                    status: MaterializationStatus::Complete,
                    applied_at: 1,
                    receipts: Vec::new(),
                });
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(JSON_WRITES.with(std::cell::Cell::get) - before_writes, 3);
        let current = repo.load_project_activity_state_snapshot().unwrap();
        assert_eq!(current.config, state.sync_config_after);
        assert_eq!(current.conversation_bindings, state.bindings_after);
        assert_eq!(current.materializations.records.len(), 3);
        assert_eq!(current.materializations.revision, before_revision + 1);
    }

    #[test]
    fn push_batch_rejects_changed_state_before_any_write() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _host = repo.acquire_host_operation().unwrap();
        for change_config in [true, false] {
            let state = push_after_state(&repo);
            if change_config {
                repo.mutate_config(|_| Ok(())).unwrap();
            } else {
                repo.mutate_conversation_bindings(|_| Ok(())).unwrap();
            }
            let before_writes = JSON_WRITES.with(std::cell::Cell::get);
            let error = repo
                .commit_conversation_publication(&state, |_| {
                    panic!("stale state must not record receipts");
                })
                .unwrap_err();
            assert!(error.contains("changed during apply"), "{error}");
            assert_eq!(JSON_WRITES.with(std::cell::Cell::get), before_writes);
        }
    }

    #[test]
    fn push_batch_receipt_failure_leaves_state_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _host = repo.acquire_host_operation().unwrap();
        let state = push_after_state(&repo);
        let before_writes = JSON_WRITES.with(std::cell::Cell::get);
        let error = repo
            .commit_conversation_publication(&state, |_| Err("receipt rejected".into()))
            .unwrap_err();
        assert_eq!(error, "receipt rejected");
        assert_eq!(JSON_WRITES.with(std::cell::Cell::get), before_writes);
        assert_eq!(repo.load_config().unwrap(), state.sync_config_before);
        assert_eq!(
            repo.load_conversation_bindings().unwrap(),
            state.bindings_before
        );
    }

    #[test]
    fn pull_batch_commits_each_session_without_reloading_documents() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _host = repo.acquire_host_operation().unwrap();
        let mut batch = repo.lock_conversation_pull().unwrap();
        let initial_reads = JSON_READS.with(std::cell::Cell::get);
        for revision in 1..=3 {
            let (mut config, bindings) = pull_after_state(&batch);
            // Change real content as well as the revision on each session.
            config.projects = vec![project()];
            config.projects[0].display_name = format!("Session {revision}");
            batch.commit(config, bindings, |_| Ok(())).unwrap();
            let current = batch.current().unwrap();
            assert_eq!(current.config.revision, revision);
            assert_eq!(current.conversation_bindings.revision, revision);
            assert_eq!(current.materializations.revision, revision);
            let config: SyncConfig =
                serde_json::from_slice(&fs::read(repo.config_path()).unwrap()).unwrap();
            let bindings: ConversationBindings =
                serde_json::from_slice(&fs::read(repo.conversation_bindings_path()).unwrap())
                    .unwrap();
            let records: Materializations =
                serde_json::from_slice(&fs::read(repo.materializations_path()).unwrap()).unwrap();
            assert_eq!(config, current.config);
            assert_eq!(bindings, current.conversation_bindings);
            assert_eq!(records, current.materializations);
        }
        assert_eq!(JSON_READS.with(std::cell::Cell::get), initial_reads);
    }

    #[test]
    fn pull_batch_rejects_stale_candidates_and_can_commit_the_next_session() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _host = repo.acquire_host_operation().unwrap();
        let mut batch = repo.lock_conversation_pull().unwrap();
        let (config, bindings) = pull_after_state(&batch);
        let stale = (config.clone(), bindings.clone());
        batch.commit(config, bindings, |_| Ok(())).unwrap();
        let before = fs::read(repo.config_path()).unwrap();
        for stale_config in [true, false] {
            let (mut config, mut bindings) = pull_after_state(&batch);
            if stale_config {
                config = stale.0.clone();
            } else {
                bindings = stale.1.clone();
            }
            assert!(batch
                .commit(config, bindings, |_| Ok(()))
                .unwrap_err()
                .contains("revision"));
            assert_eq!(batch.current().unwrap().config.revision, 1);
            assert_eq!(fs::read(repo.config_path()).unwrap(), before);
        }
        let (config, bindings) = pull_after_state(&batch);
        batch.commit(config, bindings, |_| Ok(())).unwrap();
        assert_eq!(batch.current().unwrap().conversation_bindings.revision, 2);
    }

    #[test]
    fn pull_batch_invalidates_state_after_any_document_write_failure() {
        for failed_document in ["bindings", "config", "receipts"] {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let _host = repo.acquire_host_operation().unwrap();
            let mut batch = repo.lock_conversation_pull().unwrap();
            let (config, bindings) = pull_after_state(&batch);
            batch.commit(config, bindings, |_| Ok(())).unwrap();
            let (config, bindings) = pull_after_state(&batch);
            let failed_path = match failed_document {
                "bindings" => repo.conversation_bindings_path(),
                "config" => repo.config_path(),
                _ => repo.materializations_path(),
            };
            fs::remove_file(&failed_path).unwrap();
            fs::create_dir(&failed_path).unwrap();
            assert!(batch
                .commit(config.clone(), bindings.clone(), |_| Ok(()))
                .is_err());
            assert!(batch.current().unwrap_err().contains("uncertain"));
            assert!(batch
                .commit(config, bindings, |_| Ok(()))
                .unwrap_err()
                .contains("uncertain"));
            // Writes before the fault remain committed. The batch does not
            // roll them back or claim that the failed session has a receipt.
            if failed_document == "receipts" {
                let config: SyncConfig =
                    serde_json::from_slice(&fs::read(repo.config_path()).unwrap()).unwrap();
                assert_eq!(config.revision, 2);
            }
        }
    }

    #[test]
    fn pull_batch_excludes_config_writers_and_preserves_their_revision_check() {
        use std::sync::mpsc;
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let stale = repo.load_config().unwrap();
        let _host = repo.acquire_host_operation().unwrap();
        let mut batch = repo.lock_conversation_pull().unwrap();
        std::thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (finished_tx, finished_rx) = mpsc::channel();
            let repository = &repo;
            scope.spawn(move || {
                started_tx.send(()).unwrap();
                finished_tx.send(repository.save_config(stale)).unwrap();
            });
            started_rx.recv().unwrap();
            assert!(matches!(
                finished_rx.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            let (config, bindings) = pull_after_state(&batch);
            batch.commit(config, bindings, |_| Ok(())).unwrap();
            drop(batch);
            assert!(finished_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap_err()
                .contains("expected revision 0, current 1"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn persistence_guard_repairs_non_private_lock_mode_once() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        repo.load_config().unwrap();
        let lock_path = repo.root().join("persistence.lock");
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o644)).unwrap();

        repo.load_config().unwrap();
        let repaired = fs::metadata(&lock_path).unwrap();
        assert_eq!(repaired.permissions().mode() & 0o7777, 0o600);
        let repaired_change_time = (repaired.ctime(), repaired.ctime_nsec());

        std::thread::sleep(std::time::Duration::from_millis(10));
        repo.load_config().unwrap();
        let unchanged = fs::metadata(&lock_path).unwrap();
        assert_eq!(
            (unchanged.ctime(), unchanged.ctime_nsec()),
            repaired_change_time,
            "an already-private lock must not be chmodded again"
        );
    }

    #[test]
    fn local_state_summary_measures_without_following_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::write(repo.root().join("sync_config.json"), b"0123456789").unwrap();
        fs::create_dir_all(repo.root().join("backups")).unwrap();
        fs::write(repo.root().join("backups").join("copy.bak"), b"abcd").unwrap();
        let outside = temp.path().join("huge.bin");
        fs::write(&outside, vec![0u8; 4096]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, repo.root().join("link")).unwrap();

        let summary = repo.local_state_summary().unwrap();
        assert_eq!(summary.root_path, repo.root().display().to_string());
        assert_eq!(summary.backups_bytes, 4);
        assert!(summary.file_count >= 2);
        assert!(summary.total_bytes >= 14);
        // The symlink counts as itself, never as its 4096-byte target.
        assert!(summary.total_bytes < 4096);
    }

    #[test]
    fn wipe_local_state_removes_root_and_reopen_is_fresh() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let mut config = repo.load_config().unwrap();
        config.projects.push(project());
        repo.save_config(config).unwrap();
        fs::create_dir_all(repo.root().join("backups")).unwrap();
        fs::write(repo.root().join("backups").join("copy.bak"), b"abcd").unwrap();

        let result = repo.wipe_local_state().unwrap();
        assert!(result.existed);
        assert!(result.files_removed > 0);
        assert!(result.bytes_removed > 0);
        assert!(!temp.path().join(".mallard").exists());

        let reopened = repository(&temp);
        assert!(reopened.load_config().unwrap().projects.is_empty());
        let fresh = reopened.load_or_create_host_identity().unwrap();
        assert_ne!(fresh.host_id, identity.host_id);
    }

    #[test]
    fn wipe_local_state_is_idempotent_when_root_missing() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::remove_dir_all(repo.root()).unwrap();
        // Same instance on purpose: constructing a new repository would
        // recreate the root via ensure_root, and the missing-root check must
        // run before any lock file could do the same.
        let result = repo.wipe_local_state().unwrap();
        assert_eq!(
            result,
            LocalStateWipeResult {
                existed: false,
                files_removed: 0,
                bytes_removed: 0
            }
        );
    }

    fn project() -> LocalProjectRegistration {
        LocalProjectRegistration {
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            display_name: "Project A".to_string(),
            local_alias: None,
            repository_fingerprint: None,
            recipe: BundleRecipe::default(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn install_bound_project(
        repo: &LocalStateRepository,
        temp: &tempfile::TempDir,
        identity: &HostIdentity,
    ) -> ReplicaId {
        let project_root = temp.path().join("project");
        let profile_root = temp.path().join("codex");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&profile_root).unwrap();

        let mut config = repo.load_config().unwrap();
        config.projects.push(project());
        repo.save_config(config).unwrap();

        let replica_id = ReplicaId::parse("replica-old").unwrap();
        repo.mutate_bindings(|_, machine| {
            machine.profiles.push(ProviderProfile {
                profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                provider: codex_key(),
                display_name: "Codex".to_string(),
                path: profile_root.to_string_lossy().into_owned(),
                canonical_path: profile_root.to_string_lossy().into_owned(),
                provider_config: mallard_domain::AgentProviderConfig::version_1(),
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            machine.bindings.push(ProjectBinding {
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
                project_root: project_root.to_string_lossy().into_owned(),
                canonical_project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(
                    codex_key(),
                    LocalProviderProfileId::parse("profile-a").unwrap(),
                )]),
                state: BindingState::Active,
                revision: 0,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        replica_id
    }

    #[test]
    fn persistence_stays_within_the_mallard_root() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("sync_config.json");
        fs::write(&outside, b"{\"keep\":true}\n").unwrap();
        let repo = repository(&temp);
        let saved = repo.save_config(SyncConfig::default()).unwrap();

        assert_eq!(saved.schema, LOCAL_SCHEMA);
        assert_eq!(saved.revision, 1);
        assert_eq!(fs::read(&outside).unwrap(), b"{\"keep\":true}\n");
        assert!(repo.config_path().starts_with(temp.path().join(".mallard")));
    }

    #[test]
    fn config_loader_rejects_pre_reset_schemas_without_rewriting_them() {
        for unsupported_schema in [2, 3] {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let mut document = serde_json::to_value(SyncConfig::default()).unwrap();
            document["schema"] = serde_json::Value::from(unsupported_schema);
            let bytes = serde_json::to_vec_pretty(&document).unwrap();
            fs::write(repo.config_path(), &bytes).unwrap();

            let error = repo.load_config().unwrap_err();
            assert!(
                error.contains("unsupported project-sync schema"),
                "{unsupported_schema}: {error}"
            );
            assert_eq!(fs::read(repo.config_path()).unwrap(), bytes);
        }
    }

    #[test]
    fn config_writes_are_revision_guarded_and_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let mut first = repo.load_config().unwrap();
        first.projects.push(project());
        let saved = repo.save_config(first.clone()).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(repo.load_config().unwrap(), saved);

        first.projects.clear();
        assert!(repo.save_config(first).unwrap_err().contains("changed"));
        assert_eq!(repo.load_config().unwrap().projects.len(), 1);
    }

    #[test]
    fn focused_mutation_is_atomic_at_the_document_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let returned = repo
            .mutate_config(|config| {
                config.projects.push(project());
                Ok(config.projects[0].local_project_id.clone())
            })
            .unwrap();
        assert_eq!(returned.as_str(), "project-a");
        assert_eq!(repo.load_config().unwrap().revision, 1);
    }

    #[test]
    fn project_state_snapshot_loads_related_documents() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        install_bound_project(&repo, &temp, &identity);

        let snapshot = repo.load_project_state_snapshot().unwrap();

        assert_eq!(snapshot.host_identity, identity);
        assert_eq!(snapshot.config.projects, vec![project()]);
        assert_eq!(snapshot.bindings.bindings.len(), 1);
        assert_eq!(snapshot.bindings.bindings[0].host_id, identity.host_id);
        assert_eq!(
            snapshot.bindings.bindings[0].local_project_id,
            snapshot.config.projects[0].local_project_id
        );
        assert!(snapshot.materializations.records.is_empty());
    }

    #[test]
    fn project_activity_state_snapshot_loads_all_activity_documents() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(&repo, &temp, &identity);
        let local_project_id = LocalProjectId::parse("project-a").unwrap();
        let thread_id = ThreadId::parse("thread-activity").unwrap();
        repo.mutate_conversation_bindings(|bindings| {
            bindings.bindings.push(LocalConversationBinding {
                installed_end: None,
                installed_subagents: Default::default(),
                path_origins_ref: None,
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: thread_id.clone(),
                cloud_thread_id: None,
                forked_from: None,
                preserved: None,
                root_tail_layout: None,
                provider_thread_id: None,
                materialization_state: ConversationMaterializationState::StoredOnly,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        let materialization_id = MaterializationId::parse("materialization-activity").unwrap();
        repo.mutate_materializations(|_, materializations| {
            materializations.records.push(MaterializationRecord {
                materialization_id: materialization_id.clone(),
                plan_id: PlanId::parse("plan-activity").unwrap(),
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                storage_id: StorageId::parse("storage-activity").unwrap(),
                bundle_id: project().bundle_id,
                revision: 1,
                commit_id: "commit-activity".to_string(),
                manifest_sha256: "a".repeat(64),
                binding_revision: 0,
                status: MaterializationStatus::Partial,
                applied_at: 1,
                receipts: Vec::new(),
            });
            Ok(())
        })
        .unwrap();

        let snapshot = repo.load_project_activity_state_snapshot().unwrap();

        assert_eq!(snapshot.host_identity, identity);
        assert_eq!(snapshot.config.projects, vec![project()]);
        assert_eq!(snapshot.bindings.bindings.len(), 1);
        assert_eq!(snapshot.conversation_bindings.bindings.len(), 1);
        assert_eq!(
            snapshot.conversation_bindings.bindings[0].thread_id,
            thread_id
        );
        assert_eq!(snapshot.materializations.records.len(), 1);
        assert_eq!(
            snapshot.materializations.records[0].materialization_id,
            materialization_id
        );
    }

    #[test]
    fn project_snapshots_include_the_identity_used_to_recover_host_state() {
        for activity in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let original = repo.load_or_create_host_identity().unwrap();
            let replica_id = install_bound_project(&repo, &temp, &original);
            fs::remove_file(repo.host_identity_path()).unwrap();

            let (identity, machine) = if activity {
                let snapshot = repo.load_project_activity_state_snapshot().unwrap();
                assert_eq!(
                    snapshot.conversation_bindings.host_id,
                    snapshot.host_identity.host_id
                );
                (snapshot.host_identity, snapshot.bindings)
            } else {
                let snapshot = repo.load_project_state_snapshot().unwrap();
                (snapshot.host_identity, snapshot.bindings)
            };

            assert_ne!(identity.host_id, original.host_id);
            assert_eq!(machine.bindings.len(), 1);
            assert_eq!(machine.bindings[0].host_id, identity.host_id);
            assert_ne!(machine.bindings[0].replica_id, replica_id);
            assert_eq!(machine.bindings[0].state, BindingState::Detached);
        }
    }

    #[test]
    fn project_sync_metadata_reset_forgets_derived_state_and_keeps_user_bytes_and_setup() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(&repo, &temp, &identity);
        let local_project_id = LocalProjectId::parse("project-a").unwrap();
        let storage_id = StorageId::parse("storage-a").unwrap();
        let storage_root = temp.path().join("storage");
        fs::create_dir_all(&storage_root).unwrap();
        let remote_sentinel = storage_root.join("remote-object");
        fs::write(&remote_sentinel, b"keep remote").unwrap();
        let provider_sentinel = temp.path().join("codex").join("provider-session.jsonl");
        fs::write(&provider_sentinel, b"keep provider").unwrap();

        repo.mutate_config(|config| {
            config.storages.push(local_storage_config(
                storage_id.clone(),
                "Storage A",
                &storage_root,
            ));
            config.links.push(ProjectStorageLink {
                local_project_id: local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: project().bundle_id,
                recipe: Some(BundleRecipe::default()),
                scope_bases: BTreeMap::from([(
                    SyncScopeKey::provider(codex_key()),
                    ScopeBase {
                        scope: SyncScopeKey::provider(codex_key()),
                        scope_revision: 1,
                        scope_sha256: "a".repeat(64),
                        reviewed_in: RepositoryBase {
                            revision: 1,
                            commit_id: "commit-a".to_string(),
                            manifest_sha256: "b".repeat(64),
                            manifest_size: 1,
                        },
                        binding_revision: 0,
                        last_pull_at: Some(1),
                        last_push_at: None,
                    },
                )]),
                project_content_preferences: Default::default(),
                conversation_checkpoints: BTreeMap::new(),
                pinned: true,
                created_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_conversation_bindings(|bindings| {
            bindings.bindings.push(LocalConversationBinding {
                installed_end: None,
                installed_subagents: Default::default(),
                path_origins_ref: None,
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: ThreadId::parse("thread-a").unwrap(),
                cloud_thread_id: None,
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(RootTailLayout {
                    codec: PORTABLE_TAIL_CODEC_V1.to_string(),
                    partition_kind: TailPartitionKind::WholeThread,
                    start_record_count: 0,
                }),
                provider_thread_id: Some("provider-thread-a".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_materializations(|_, materializations| {
            materializations.records.push(MaterializationRecord {
                materialization_id: MaterializationId::parse("materialization-a").unwrap(),
                plan_id: PlanId::parse("plan-a").unwrap(),
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: project().bundle_id,
                revision: 1,
                commit_id: "commit-a".to_string(),
                manifest_sha256: "c".repeat(64),
                binding_revision: 0,
                status: MaterializationStatus::Partial,
                applied_at: 1,
                receipts: Vec::new(),
            });
            Ok(())
        })
        .unwrap();

        let result = repo
            .reset_project_sync_metadata(&local_project_id, 0)
            .unwrap();
        assert_eq!(result.links_reset, 1);
        assert_eq!(result.scope_bases_removed, 1);
        assert_eq!(result.conversation_bindings_removed, 1);
        assert_eq!(result.materializations_removed, 1);

        let config = repo.load_config().unwrap();
        assert!(config.project(&local_project_id).is_some());
        assert_eq!(config.storages.len(), 1);
        let link = config
            .links
            .iter()
            .find(|link| link.local_project_id == local_project_id)
            .unwrap();
        assert!(link.scope_bases.is_empty());
        assert!(link.conversation_checkpoints.is_empty());
        assert!(link.recipe.is_some());
        assert!(link.pinned);
        assert!(repo
            .load_bindings()
            .unwrap()
            .active_for(&local_project_id)
            .is_some());
        assert!(repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .is_empty());
        assert!(repo.load_materializations().unwrap().records.is_empty());
        assert_eq!(fs::read(remote_sentinel).unwrap(), b"keep remote");
        assert_eq!(fs::read(provider_sentinel).unwrap(), b"keep provider");

        let second = repo
            .reset_project_sync_metadata(&local_project_id, 0)
            .unwrap();
        assert_eq!(second.scope_bases_removed, 0);
        assert_eq!(second.conversation_bindings_removed, 0);
        assert_eq!(second.materializations_removed, 0);
    }

    #[test]
    fn storage_clear_keeps_local_conversation_preservation_provenance() {
        use mallard_domain::{
            AcceptedSubagentState, InstalledSubagent, SubagentContentState, SubagentKey,
        };

        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(&repo, &temp, &identity);
        let local_project_id = LocalProjectId::parse("project-a").unwrap();
        let storage_id = StorageId::parse("storage-a").unwrap();
        let logical_thread_id = ThreadId::parse("thread-preserved").unwrap();
        let observed_tail = mallard_domain::compute_portable_conversation_tail(
            TailPartitionKind::WholeThread,
            0,
            &[CanonicalConversationMessage {
                message_id: MessageId::parse("message-preserved").unwrap(),
                parent_message_id: None,
                semantic_fingerprint: "f".repeat(64),
            }],
        )
        .unwrap();
        let child_key = SubagentKey::parse("start-child").unwrap();
        let child_state = SubagentContentState::from_tail(&observed_tail).unwrap();
        repo.mutate_config(|config| {
            config.storages.push(local_storage_config(
                storage_id.clone(),
                "Storage A",
                &temp.path().join("storage"),
            ));
            config.links.push(ProjectStorageLink {
                local_project_id: local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: project().bundle_id,
                recipe: Some(BundleRecipe::default()),
                scope_bases: BTreeMap::from([(
                    SyncScopeKey::provider(codex_key()),
                    ScopeBase {
                        scope: SyncScopeKey::provider(codex_key()),
                        scope_revision: 1,
                        scope_sha256: "c".repeat(64),
                        reviewed_in: RepositoryBase {
                            revision: 1,
                            commit_id: "commit-clear".to_string(),
                            manifest_sha256: "d".repeat(64),
                            manifest_size: 1,
                        },
                        binding_revision: 0,
                        last_pull_at: Some(1),
                        last_push_at: None,
                    },
                )]),
                project_content_preferences: Default::default(),
                conversation_checkpoints: BTreeMap::from([(
                    SyncScopeKey::provider(codex_key()),
                    BTreeMap::from([(
                        logical_thread_id.clone(),
                        ConversationCheckpoint {
                            accepted_subagents: BTreeMap::from([(
                                child_key.clone(),
                                AcceptedSubagentState {
                                    local: child_state.clone(),
                                    normalized: child_state.clone(),
                                },
                            )]),
                            generation: 5,
                            accepted: Some(AcceptedConversationVersion {
                                artifact_sha256: "a".repeat(64),
                                accepted_at_commit_id: "commit-clear".to_string(),
                                accepted_at_manifest_sha256: "d".repeat(64),
                                portable_tail: observed_tail.clone(),
                            }),
                            accepted_local_tail: Some(observed_tail.clone()),
                            observed_tail: observed_tail.clone(),
                            physical: PhysicalMappingState::Detached {
                                expected_provider_thread_id: "provider-thread-preserved"
                                    .to_string(),
                                expected_binding_revision: 0,
                            },
                            prefix_witness: None,
                            binding_revision: 0,
                        },
                    )]),
                )]),
                pinned: true,
                created_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_conversation_bindings(|bindings| {
            bindings.bindings.push(LocalConversationBinding {
                installed_end: Some(128),
                installed_subagents: BTreeMap::from([(
                    child_key,
                    InstalledSubagent {
                        provider_thread_id: "provider-child-preserved".to_string(),
                        installed_end: 64,
                    },
                )]),
                path_origins_ref: None,
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: logical_thread_id.clone(),
                cloud_thread_id: None,
                forked_from: None,
                preserved: Some(PreservedProvenance {
                    preserved_from: mallard_domain::CloudThreadId::parse("cloudthread-origin")
                        .unwrap(),
                    ordinal: 3,
                }),
                root_tail_layout: Some(RootTailLayout {
                    codec: PORTABLE_TAIL_CODEC_V1.to_string(),
                    partition_kind: TailPartitionKind::WholeThread,
                    start_record_count: 0,
                }),
                provider_thread_id: Some("provider-thread-preserved".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_materializations(|_, materializations| {
            materializations.records.push(MaterializationRecord {
                materialization_id: MaterializationId::parse("materialization-a").unwrap(),
                plan_id: PlanId::parse("plan-a").unwrap(),
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: project().bundle_id,
                revision: 1,
                commit_id: "commit-a".to_string(),
                manifest_sha256: "c".repeat(64),
                binding_revision: 0,
                status: MaterializationStatus::Partial,
                applied_at: 1,
                receipts: Vec::new(),
            });
            Ok(())
        })
        .unwrap();

        let expected_config_revision = repo.load_config().unwrap().revision;
        let config_before = repo.load_config().unwrap();
        let bindings_before = repo.load_conversation_bindings().unwrap();
        let materializations_before = repo.load_materializations().unwrap();
        let stale = repo
            .clear_storage_data(
                &storage_id,
                expected_config_revision + 1,
                |_| -> Result<(), String> {
                    panic!("stale configuration must not start remote deletion");
                },
            )
            .unwrap_err();
        assert!(stale.contains("storage configuration changed"));
        let failed = repo
            .clear_storage_data(
                &storage_id,
                expected_config_revision,
                |_| -> Result<(), String> {
                    Err("Some data was deleted. Retry Clear storage data.".to_string())
                },
            )
            .unwrap_err();
        assert!(failed.contains("Some data was deleted"), "{failed}");
        assert_eq!(repo.load_config().unwrap(), config_before);
        assert_eq!(
            repo.load_materializations().unwrap(),
            materializations_before
        );
        let (_, result) = repo
            .clear_storage_data(&storage_id, expected_config_revision, |_| {
                Ok::<_, String>(())
            })
            .unwrap();

        assert_eq!(result.materializations_removed, 1);
        assert_eq!(result.scope_bases_removed, 1);
        assert_eq!(result.conversation_acceptances_removed, 1);
        assert!(repo.load_materializations().unwrap().records.is_empty());
        let bindings = repo.load_conversation_bindings().unwrap();
        assert_eq!(bindings, bindings_before);
        assert_eq!(bindings.bindings.len(), 1);
        assert_eq!(
            bindings.bindings[0].preserved,
            Some(PreservedProvenance {
                preserved_from: mallard_domain::CloudThreadId::parse("cloudthread-origin").unwrap(),
                ordinal: 3,
            })
        );
        let config = repo.load_config().unwrap();
        let link = config
            .links
            .iter()
            .find(|link| link.local_project_id == local_project_id && link.storage_id == storage_id)
            .unwrap();
        assert!(link.scope_bases.is_empty());
        let checkpoint = &link.conversation_checkpoints[&SyncScopeKey::provider(codex_key())]
            [&logical_thread_id];
        assert_eq!(checkpoint.generation, 6);
        assert!(checkpoint.accepted.is_none());
        assert!(checkpoint.accepted_local_tail.is_none());
        assert!(checkpoint.accepted_subagents.is_empty());
        assert_eq!(checkpoint.observed_tail, observed_tail);
        assert!(matches!(
            checkpoint.physical,
            PhysicalMappingState::Detached {
                ref expected_provider_thread_id,
                expected_binding_revision: 0,
            } if expected_provider_thread_id == "provider-thread-preserved"
        ));
    }

    #[test]
    fn project_sync_metadata_reset_rejects_a_stale_project_revision_before_writes() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let (binding, _) = bound_path_origin_fixture(&repo, &temp);
        repo.mutate_conversation_bindings(|document| {
            document.bindings.push(binding);
            Ok(())
        })
        .unwrap();
        let bindings_before = repo.load_conversation_bindings().unwrap();
        let before = repo.load_config().unwrap();

        let error = repo
            .reset_project_sync_metadata(
                &LocalProjectId::parse("project-a").unwrap(),
                before.projects[0].revision.saturating_add(1),
            )
            .unwrap_err();

        assert!(error.contains("project changed"));
        assert_eq!(repo.load_config().unwrap(), before);
        assert_eq!(repo.load_conversation_bindings().unwrap(), bindings_before);
    }

    #[test]
    fn config_transaction_returns_the_exact_committed_document() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let (project_id, committed) = repo
            .transact_config(|config| {
                config.projects.push(project());
                Ok::<_, String>(ConfigMutation::Changed(
                    config.projects[0].local_project_id.clone(),
                ))
            })
            .unwrap_or_else(|error| match error {
                ConfigTransactionError::Mutation(error)
                | ConfigTransactionError::Persistence(error) => panic!("{error}"),
            });

        assert_eq!(project_id.as_str(), "project-a");
        assert_eq!(committed.revision, 1);
        assert_eq!(committed.projects.len(), 1);
        assert_eq!(repo.load_config().unwrap(), committed);
    }

    #[test]
    fn unchanged_config_transaction_is_idempotent_without_a_revision_bump() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let first = repo.save_config(SyncConfig::default()).unwrap();

        let (value, observed) = repo
            .transact_config(|_| Ok::<_, String>(ConfigMutation::Unchanged("already-present")))
            .unwrap_or_else(|error| match error {
                ConfigTransactionError::Mutation(error)
                | ConfigTransactionError::Persistence(error) => panic!("{error}"),
            });

        assert_eq!(value, "already-present");
        assert_eq!(observed, first);
        assert_eq!(repo.load_config().unwrap().revision, 1);
    }

    #[test]
    fn project_bundle_transaction_recovers_each_interrupted_write() {
        for completed_writes in 0..=2 {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let identity = repo.load_or_create_host_identity().unwrap();
            let replica_id = install_bound_project(&repo, &temp, &identity);
            repo.mutate_conversation_bindings(|conversations| {
                conversations.bindings.push(LocalConversationBinding {
                    installed_end: None,
                    installed_subagents: Default::default(),
                    path_origins_ref: None,
                    host_id: identity.host_id.clone(),
                    replica_id: replica_id.clone(),
                    local_project_id: LocalProjectId::parse("project-a").unwrap(),
                    provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                    thread_id: ThreadId::parse("group-a.variant-a").unwrap(),
                    cloud_thread_id: None,
                    forked_from: None,
                    preserved: None,
                    root_tail_layout: None,
                    provider_thread_id: None,
                    materialization_state: ConversationMaterializationState::StoredOnly,
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                Ok(())
            })
            .unwrap();
            let before = repo.load_project_activity_state_snapshot().unwrap();
            let mut config = before.config.clone();
            let mut machine = before.bindings.clone();
            let bundle_id = BundleId::parse("ffffffffffffffffffffffffffffffff").unwrap();
            config.projects[0].bundle_id = bundle_id.clone();
            config.projects[0].revision += 1;
            config.revision += 1;
            machine.bindings[0].bundle_id = bundle_id.clone();
            machine.bindings[0].revision += 1;
            machine.revision += 1;
            let transaction = ProjectBundleTransaction { config, machine };
            let journal = repo.root.join("project_bundle_transaction.json");
            write_json_atomic(
                &repo.root,
                &journal,
                &transaction,
                2 * (MAX_CONFIG_BYTES + MAX_BINDINGS_BYTES),
            )
            .unwrap();
            if completed_writes >= 1 {
                write_json_atomic(
                    &repo.root,
                    &repo.machine_projects_path(),
                    &transaction.machine,
                    MAX_BINDINGS_BYTES,
                )
                .unwrap();
            }
            if completed_writes >= 2 {
                write_json_atomic(
                    &repo.root,
                    &repo.config_path(),
                    &transaction.config,
                    MAX_CONFIG_BYTES,
                )
                .unwrap();
            }
            let recovered = repo.load_project_activity_state_snapshot().unwrap();
            assert_eq!(recovered.config, transaction.config);
            assert_eq!(recovered.bindings, transaction.machine);
            assert_eq!(
                recovered.conversation_bindings,
                before.conversation_bindings
            );
            assert_eq!(recovered.bindings.bindings[0].replica_id, replica_id);
            assert_eq!(recovered.config.schema, 1);
            assert_eq!(recovered.bindings.schema, 1);
            assert!(!journal.exists());
            let error = repo
                .mutate_project_bundle(
                    &LocalProjectId::parse("project-a").unwrap(),
                    before.config.revision,
                    |_| panic!("stale config must be rejected before mutation"),
                )
                .unwrap_err();
            assert!(error.contains("configuration changed"));
            assert_eq!(repo.load_config().unwrap(), recovered.config);
        }
    }

    #[test]
    fn machine_mutations_cannot_strand_conversation_scope() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(&repo, &temp, &identity);
        let second_profile_root = temp.path().join("codex-second");
        fs::create_dir_all(&second_profile_root).unwrap();
        let first_profile_id = LocalProviderProfileId::parse("profile-a").unwrap();
        let second_profile_id = LocalProviderProfileId::parse("profile-b").unwrap();
        repo.mutate_bindings(|_, machine| {
            machine.profiles.push(ProviderProfile {
                profile_id: second_profile_id.clone(),
                provider: codex_key(),
                display_name: "Second Codex".to_string(),
                path: second_profile_root.to_string_lossy().into_owned(),
                canonical_path: second_profile_root.to_string_lossy().into_owned(),
                provider_config: mallard_domain::AgentProviderConfig::version_1(),
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_conversation_bindings(|conversations| {
            conversations.bindings.push(LocalConversationBinding {
                installed_end: None,
                installed_subagents: Default::default(),
                path_origins_ref: None,
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                provider_profile_id: first_profile_id.clone(),
                thread_id: ThreadId::parse("group-a.variant-a").unwrap(),
                cloud_thread_id: None,
                forked_from: None,
                preserved: None,
                root_tail_layout: None,
                provider_thread_id: None,
                materialization_state: ConversationMaterializationState::StoredOnly,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_bindings(|_, machine| {
            machine.bindings[0].state = BindingState::Detached;
            Ok(())
        })
        .unwrap();

        let error = repo
            .mutate_bindings(|_, machine| {
                machine.bindings[0].profile_ids =
                    BTreeMap::from([(codex_key(), second_profile_id.clone())]);
                Ok(())
            })
            .unwrap_err();
        assert!(error.contains("scope is stale"));
        let after_profile_change = repo.load_bindings().unwrap();
        assert_eq!(
            after_profile_change.bindings[0].profile_ids[&codex_key()],
            first_profile_id
        );

        let error = repo
            .mutate_bindings(|_, machine| {
                machine
                    .profiles
                    .retain(|profile| profile.profile_id != first_profile_id);
                Ok(())
            })
            .unwrap_err();
        assert!(error.contains("scope is stale"));
        assert!(repo
            .load_bindings()
            .unwrap()
            .profiles
            .iter()
            .any(|profile| profile.profile_id == first_profile_id));
        repo.load_conversation_bindings().unwrap();
    }

    #[test]
    fn metadata_root_creation_accepts_a_concurrent_winner() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".mallard");
        fs::create_dir(&root).unwrap();

        create_metadata_root(&root).unwrap();
        assert!(root.is_dir());
    }

    #[test]
    fn host_sensitive_workflows_are_single_flight_per_repository() {
        let temp = tempfile::tempdir().unwrap();
        let first = repository(&temp);
        let second = repository(&temp);
        let guard = first.acquire_host_operation().unwrap();
        assert!(second
            .acquire_host_operation()
            .unwrap_err()
            .contains("already in progress"));
        drop(guard);
        second.acquire_host_operation().unwrap();
    }

    #[test]
    fn dropping_host_operation_guard_unlocks_a_duplicated_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let first = repository(&temp);
        let second = repository(&temp);
        let guard = first.acquire_host_operation().unwrap();
        let duplicated = guard._file.try_clone().unwrap();

        drop(guard);
        second.acquire_host_operation().unwrap();
        drop(duplicated);
    }

    #[cfg(unix)]
    #[test]
    fn metadata_root_must_not_be_a_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), temp.path().join(".mallard")).unwrap();
        let error = LocalStateRepository::from_home_dir(temp.path()).unwrap_err();
        assert!(error.contains("not a real directory"));
    }

    #[test]
    fn host_identity_is_stable_private_and_bundle_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let first = repo.load_or_create_host_identity().unwrap();
        let second = repo.load_or_create_host_identity().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.host_id.as_str().len(), 32);
        let identity_json = fs::read_to_string(repo.host_identity_path()).unwrap();
        assert!(identity_json.contains(first.host_id.as_str()));

        let bundle_a = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let bundle_b = BundleId::parse("11111111111111111111111111111111").unwrap();
        let replica = ReplicaId::parse("replica-a").unwrap();
        let a = repo.derive_origin_tokens(&bundle_a, &replica).unwrap();
        assert_eq!(a, repo.derive_origin_tokens(&bundle_a, &replica).unwrap());
        assert_ne!(
            a.0,
            repo.derive_origin_tokens(&bundle_b, &replica).unwrap().0
        );
        assert_ne!(a.0.as_str(), first.host_id.as_str());
        assert_ne!(a.1.as_str(), replica.as_str());
        let second_replica = repo
            .derive_origin_tokens(&bundle_a, &ReplicaId::parse("replica-b").unwrap())
            .unwrap();
        assert_eq!(a.0, second_replica.0);
        assert_ne!(a.1, second_replica.1);

        let second_temp = tempfile::tempdir().unwrap();
        let second_repo = repository(&second_temp);
        let other = second_repo.load_or_create_host_identity().unwrap();
        assert_ne!(first.host_id, other.host_id);
        assert_ne!(
            second_repo
                .derive_origin_tokens(&bundle_a, &replica)
                .unwrap()
                .0,
            a.0
        );
    }

    #[test]
    fn a_missing_identity_auto_registers_a_fresh_host() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        repo.mutate_conversation_bindings(|_| Ok(())).unwrap();
        fs::remove_file(repo.host_identity_path()).unwrap();

        let registered = repo.load_or_create_host_identity().unwrap();
        assert_ne!(registered.host_id, original.host_id);
        assert_eq!(repo.load_or_create_host_identity().unwrap(), registered);
        assert_eq!(
            repo.load_conversation_bindings().unwrap().host_id,
            registered.host_id
        );
        assert!(!repo.host_rotation_path().exists());
    }

    #[test]
    fn auto_registration_detaches_bindings_under_a_fresh_host() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        let old_replica = install_bound_project(&repo, &temp, &original);
        fs::remove_file(repo.host_identity_path()).unwrap();

        let registered = repo.load_or_create_host_identity().unwrap();
        assert_ne!(registered.host_id, original.host_id);
        let machine = repo.load_bindings().unwrap();
        assert_eq!(machine.bindings.len(), 1);
        let binding = &machine.bindings[0];
        assert_eq!(binding.host_id, registered.host_id);
        assert_ne!(binding.replica_id, old_replica);
        assert_eq!(binding.state, BindingState::Detached);
    }

    #[test]
    fn semantically_invalid_host_scoped_state_is_quarantined_by_auto_registration() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::write(repo.conversation_bindings_path(), b"{}\n").unwrap();

        let registered = repo.load_or_create_host_identity().unwrap();
        assert_eq!(
            repo.load_conversation_bindings().unwrap().host_id,
            registered.host_id
        );
        assert!(repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .is_empty());
        assert!(fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("conversation-bindings-")));
    }

    #[test]
    fn loading_bindings_rejects_a_structurally_valid_foreign_host_binding() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        install_bound_project(&repo, &temp, &identity);
        let mut foreign = repo.load_bindings().unwrap();
        foreign.bindings[0].host_id = HostId::generate().unwrap();
        foreign.validate(&repo.load_config().unwrap()).unwrap();
        write_json_atomic(
            repo.root(),
            &repo.machine_projects_path(),
            &foreign,
            MAX_BINDINGS_BYTES,
        )
        .unwrap();
        let preserved = fs::read(repo.machine_projects_path()).unwrap();

        let error = repo.load_bindings().unwrap_err();
        assert!(error.contains("different host"), "{error}");
        assert_eq!(fs::read(repo.machine_projects_path()).unwrap(), preserved);
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(identity)
        );
        assert!(!repo.host_rotation_path().exists());
    }

    #[test]
    fn an_invalid_rotation_journal_is_quarantined_on_load() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked().unwrap();
        let transaction = HostRotationTransaction {
            // The payload of an unsupported schema must never be replayed.
            schema: 99,
            next_identity: next,
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(previous.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine::default(),
            created_at: 1,
        };
        write_json_atomic(
            repo.root(),
            &repo.host_rotation_path(),
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        )
        .unwrap();

        // Loading quarantines the invalid journal and keeps the live identity.
        assert_eq!(repo.load_or_create_host_identity().unwrap(), previous);
        assert!(!repo.host_rotation_path().exists());
        assert!(fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("host-rotation-")));
    }

    #[cfg(unix)]
    #[test]
    fn transient_rotation_journal_io_failure_is_not_quarantined_as_corruption() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked().unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine::default(),
            created_at: 1,
        };
        write_json_atomic(
            repo.root(),
            &repo.host_rotation_path(),
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        )
        .unwrap();
        let path = repo.host_rotation_path();
        let preserved = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let result = repo.load_or_create_host_identity();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let error = result.unwrap_err();
        assert!(error.contains("open"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), preserved);
        assert!(!repo.root().join("host_recovery_quarantine").exists());
    }

    #[test]
    fn auto_registration_quarantines_unreadable_rotation_and_setup_journals() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        repo.mutate_conversation_bindings(|_| Ok(())).unwrap();
        fs::write(repo.host_rotation_path(), b"{not-json").unwrap();
        let setup = repo.root().join("setup_transactions").join("broken.json");
        repo.ensure_directory(setup.parent().unwrap()).unwrap();
        fs::write(&setup, b"{\"schema\":2}\n").unwrap();
        fs::remove_file(repo.host_identity_path()).unwrap();

        let registered = repo.load_or_create_host_identity().unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert!(!setup.exists());
        assert!(!repo.host_rotation_path().exists());
        let names = fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name.starts_with("host-rotation-")));
        assert!(names
            .iter()
            .any(|name| name.starts_with("setup-transaction-")));
    }

    #[test]
    fn nested_setup_quarantine_retry_preserves_source_until_destination_is_durable() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let setup = repo.root().join("setup_transactions").join("broken.json");
        repo.ensure_directory(setup.parent().unwrap()).unwrap();
        let malformed = b"{broken-setup";
        fs::write(&setup, malformed).unwrap();
        let quarantine = repo.root().join("host_recovery_quarantine");
        let mut failed_destination_sync = false;

        let error = repo
            .quarantine_file_with_sync_unlocked(&setup, "setup-transaction", |directory| {
                if directory == quarantine && !failed_destination_sync {
                    failed_destination_sync = true;
                    return Err("injected quarantine destination sync failure".to_string());
                }
                sync_directory(directory)
            })
            .unwrap_err();
        assert!(error.contains("injected"), "{error}");
        assert_eq!(fs::read(&setup).unwrap(), malformed);
        assert_eq!(fs::read_dir(&quarantine).unwrap().count(), 1);

        // A whole-operation retry still discovers the live source. It may
        // leave the earlier preserved link as a harmless duplicate, but it
        // cannot lose the only copy of the malformed journal.
        repo.quarantine_file_unlocked(&setup, "setup-transaction")
            .unwrap();
        assert!(!setup.exists());
        let preserved = fs::read_dir(&quarantine)
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(preserved.len(), 2);
        assert!(preserved.iter().all(|bytes| bytes == malformed));
    }

    #[test]
    fn auto_registration_quarantines_damaged_live_state() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        fs::write(repo.machine_projects_path(), b"{broken-machine").unwrap();
        fs::write(repo.materializations_path(), b"{broken-materializations").unwrap();

        fs::remove_file(repo.host_identity_path()).unwrap();
        let registered = repo.load_or_create_host_identity().unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert!(repo.load_bindings().unwrap().bindings.is_empty());
        assert!(repo.load_materializations().unwrap().records.is_empty());
        let names = fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(names
            .iter()
            .any(|name| name.starts_with("machine-projects-")));
        assert!(names
            .iter()
            .any(|name| name.starts_with("materializations-")));
    }

    #[test]
    fn recovery_replays_journaled_quarantine_before_retiring_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked().unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine {
                machine_projects: true,
                conversation_bindings: true,
                materializations: true,
            },
            created_at: 1,
        };
        repo.validate_host_rotation_payload_unlocked(&transaction, &repo.load_config().unwrap())
            .unwrap();
        for (path, bytes) in [
            (repo.machine_projects_path(), b"{broken-machine".as_slice()),
            (
                repo.conversation_bindings_path(),
                b"{broken-conversations".as_slice(),
            ),
            (
                repo.materializations_path(),
                b"{broken-materializations".as_slice(),
            ),
        ] {
            fs::write(path, bytes).unwrap();
        }
        write_json_atomic(
            repo.root(),
            &repo.host_rotation_path(),
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        )
        .unwrap();
        let machine_quarantine =
            repo.host_rotation_quarantine_destination(&transaction, "machine-projects");
        repo.ensure_directory(machine_quarantine.parent().unwrap())
            .unwrap();
        fs::hard_link(repo.machine_projects_path(), &machine_quarantine).unwrap();
        assert!(repo.machine_projects_path().is_file());
        assert!(machine_quarantine.is_file());
        // Resume a failed destination-directory sync. Both names refer to the
        // same preserved inode, so removing the live name is not a collision.
        repo.quarantine_host_rotation_file_unlocked(
            &transaction,
            &repo.machine_projects_path(),
            "machine-projects",
        )
        .unwrap();
        assert!(!repo.machine_projects_path().exists());
        assert!(machine_quarantine.is_file());

        // Recovery sees a missing live source and must confirm the exact
        // journal-derived preserved destination before replacing the document.
        repo.recover_host_rotation_unlocked().unwrap();
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(next.clone())
        );
        assert!(repo.load_bindings().unwrap().bindings.is_empty());
        assert!(repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .is_empty());
        assert!(repo.load_materializations().unwrap().records.is_empty());
        assert!(!repo.host_rotation_path().exists());
        for category in [
            "machine-projects",
            "conversation-bindings",
            "materializations",
        ] {
            assert!(
                repo.host_rotation_quarantine_destination(&transaction, category)
                    .is_file(),
                "{category}"
            );
        }
    }

    #[test]
    fn journaled_quarantine_does_not_accept_a_missing_unpreserved_source() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let _previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked().unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine {
                machine_projects: true,
                ..HostRotationQuarantine::default()
            },
            created_at: 1,
        };

        fs::remove_file(repo.machine_projects_path()).ok();
        let error = repo
            .quarantine_host_rotation_file_unlocked(
                &transaction,
                &repo.machine_projects_path(),
                "machine-projects",
            )
            .unwrap_err();
        assert!(error.contains("confirm preserved"), "{error}");
    }

    #[test]
    fn a_malformed_identity_is_preserved_and_replaced_automatically() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        fs::write(repo.host_identity_path(), b"{not-json").unwrap();

        let registered = repo.load_or_create_host_identity().unwrap();
        assert_ne!(registered.host_id, original.host_id);
        assert_eq!(repo.load_or_create_host_identity().unwrap(), registered);
        assert!(fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("host-identity-")));
    }

    #[cfg(unix)]
    #[test]
    fn transient_identity_io_failure_is_not_quarantined_as_corruption() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let path = repo.host_identity_path();
        let preserved = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let result = repo.load_or_create_host_identity();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let error = result.unwrap_err();
        assert!(error.contains("open"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), preserved);
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(identity)
        );
        assert!(!repo.root().join("host_recovery_quarantine").exists());
    }

    fn storage_setup_draft(secret: &str) -> mallard_domain::StorageSetupDraft {
        mallard_domain::StorageSetupDraft {
            schema: mallard_domain::STORAGE_SETUP_DRAFT_SCHEMA,
            draft_id: mallard_domain::StorageSetupDraftId::parse(
                generated_named_id("storagedraft").unwrap(),
            )
            .unwrap(),
            source: mallard_domain::StorageSetupDraftSource::New,
            storage: StorageConfig {
                id: StorageId::parse(generated_named_id("storage").unwrap()).unwrap(),
                name: "Half-typed R2".to_string(),
                provider: mallard_domain::StorageProviderKey::parse("r2").unwrap(),
                provider_config: mallard_domain::StorageProviderConfig::new(
                    1,
                    mallard_domain::BoundedConfigObject::from_json(&serde_json::json!({
                        "access_key_id": "drafted-access",
                    }))
                    .unwrap(),
                    {
                        let mut secrets = mallard_domain::SecretMap::empty();
                        if !secret.is_empty() {
                            secrets
                                .insert(
                                    mallard_domain::SecretKey::parse("secret_access_key").unwrap(),
                                    mallard_domain::SecretValue::parse(secret).unwrap(),
                                )
                                .unwrap();
                        }
                        secrets
                    },
                ),
            },
            revision: 0,
            created_at: 7,
            updated_at: 7,
        }
    }

    #[test]
    fn storage_setup_drafts_roundtrip_with_revision_guard_and_private_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let draft = storage_setup_draft("draft-secret-canary");

        let saved = repo.save_storage_setup_draft(draft.clone()).unwrap();
        assert_eq!(saved.revision, 1);
        let path = repo.storage_setup_draft_path(&saved.draft_id);
        assert!(path.starts_with(repo.root().join("storage_drafts")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let loaded = repo
            .load_storage_setup_draft(&saved.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, saved);
        // The secret is persisted on disk — the draft is the only copy of a
        // key Cloudflare shows once.  Views redact it before IPC.
        assert_eq!(
            loaded
                .storage
                .provider_config
                .secrets
                .get(&mallard_domain::SecretKey::parse("secret_access_key").unwrap())
                .unwrap()
                .expose(),
            "draft-secret-canary"
        );

        let stale = repo.save_storage_setup_draft(loaded.clone()).unwrap();
        assert_eq!(stale.revision, 2);
        let error = repo.save_storage_setup_draft(loaded).unwrap_err();
        assert!(error.contains("storage draft changed"));

        let mut not_new = storage_setup_draft("");
        not_new.revision = 3;
        assert!(repo
            .save_storage_setup_draft(not_new)
            .unwrap_err()
            .contains("must start at revision 0"));
    }

    #[test]
    fn storage_setup_draft_creation_is_capped_but_updates_are_not() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let mut first = None;
        for _ in 0..mallard_domain::MAX_STORAGE_SETUP_DRAFTS {
            let saved = repo
                .save_storage_setup_draft(storage_setup_draft(""))
                .unwrap();
            first.get_or_insert(saved);
        }
        let error = repo
            .save_storage_setup_draft(storage_setup_draft(""))
            .unwrap_err();
        assert!(error.contains("too many storage drafts"));

        let mut update = first.unwrap();
        update.storage.name = "renamed at the cap".to_string();
        assert_eq!(repo.save_storage_setup_draft(update).unwrap().revision, 2);
    }

    #[test]
    fn storage_setup_draft_discard_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let saved = repo
            .save_storage_setup_draft(storage_setup_draft(""))
            .unwrap();
        assert!(repo.delete_storage_setup_draft(&saved.draft_id).unwrap());
        assert!(!repo.delete_storage_setup_draft(&saved.draft_id).unwrap());
        let unknown =
            mallard_domain::StorageSetupDraftId::parse(generated_named_id("storagedraft").unwrap())
                .unwrap();
        assert!(!repo.delete_storage_setup_draft(&unknown).unwrap());
    }

    #[test]
    fn storage_setup_draft_listing_reports_value_free_warnings_for_unreadable_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let readable = repo
            .save_storage_setup_draft(storage_setup_draft("kept-secret"))
            .unwrap();
        let mut older = storage_setup_draft("");
        older.created_at = 1;
        older.updated_at = 1;
        let older = repo.save_storage_setup_draft(older).unwrap();
        // A wrong-typed field makes serde echo the value in its message;
        // the listing must never forward that text.
        fs::write(
            repo.root()
                .join("storage_drafts")
                .join("storagedraft-000000000000000000000000.json"),
            br#"{"schema":1,"draft_id":"storagedraft-000000000000000000000000","revision":"leaked-secret-canary","storage":{},"created_at":1,"updated_at":1}"#,
        )
        .unwrap();

        let (drafts, warnings) = repo.list_storage_setup_drafts().unwrap();
        assert_eq!(
            drafts
                .iter()
                .map(|draft| draft.draft_id.clone())
                .collect::<Vec<_>>(),
            vec![readable.draft_id, older.draft_id],
            "readable drafts stay listed, newest first"
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unreadable storage draft"));
        for warning in &warnings {
            assert!(!warning.contains("leaked-secret-canary"));
        }
    }

    #[test]
    fn storage_setup_draft_file_identity_must_match_its_name() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let saved = repo
            .save_storage_setup_draft(storage_setup_draft(""))
            .unwrap();
        let imposter =
            mallard_domain::StorageSetupDraftId::parse(generated_named_id("storagedraft").unwrap())
                .unwrap();
        fs::copy(
            repo.storage_setup_draft_path(&saved.draft_id),
            repo.storage_setup_draft_path(&imposter),
        )
        .unwrap();
        let error = repo.load_storage_setup_draft(&imposter).unwrap_err();
        assert!(error.contains("contains draft"));
    }
    fn bound_path_origin_fixture(
        repo: &LocalStateRepository,
        temp: &tempfile::TempDir,
    ) -> (
        LocalConversationBinding,
        crate::session_path_origins::PathOrigins,
    ) {
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(repo, temp, &identity);
        let mut binding = LocalConversationBinding {
            installed_end: None,
            installed_subagents: Default::default(),
            host_id: identity.host_id,
            replica_id,
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
            thread_id: ThreadId::parse("thread-origins").unwrap(),
            provider_thread_id: Some("physical-origins".into()),
            cloud_thread_id: None,
            path_origins_ref: None,
            forked_from: None,
            preserved: None,
            root_tail_layout: Some(mallard_domain::RootTailLayout {
                codec: mallard_domain::PORTABLE_TAIL_CODEC_V1.into(),
                partition_kind: TailPartitionKind::WholeThread,
                start_record_count: 0,
            }),
            materialization_state: ConversationMaterializationState::Resumable,
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };
        let record = serde_json::json!({"text":"/tmp/source/file /tmp/dest/literal 🦆"});
        let encoded = crate::session_path_map::SessionPathMapper::new("/tmp/source", None, None)
            .unwrap()
            .encode_record_with_origins(&record, None)
            .unwrap();
        let mut origins = crate::session_path_origins::PathOrigins::default();
        origins.push("physical-origins", encoded.origins);
        let owner = crate::session_path_origins::PathOriginsOwner::for_binding(&binding).unwrap();
        binding.path_origins_ref = Some(repo.store_path_origins(owner, &origins).unwrap());
        (binding, origins)
    }

    #[test]
    fn path_origins_are_immutable_private_owned_and_hash_verified() {
        use crate::session_path_origins::PathOriginsOwner;
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let (binding, origins) = bound_path_origin_fixture(&repo, &temp);
        let reference = binding.path_origins_ref.as_ref().unwrap();
        assert_eq!(repo.load_path_origins(&binding).unwrap(), origins);
        assert_eq!(
            &repo
                .store_path_origins(PathOriginsOwner::for_binding(&binding).unwrap(), &origins)
                .unwrap(),
            reference
        );
        let path = repo
            .root
            .join("path_origins")
            .join(format!("{}.json", reference.sha256));
        let original = fs::read(&path).unwrap();
        assert_eq!(original.len() as u64, reference.size_bytes);
        let document: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(document["schema"], 1);
        assert!(!String::from_utf8_lossy(&original).contains("/tmp/source"));
        assert!(!String::from_utf8_lossy(&original).contains("/tmp/dest/literal"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // Keep both can change the logical ID while retaining one physical history.
        let mut other = binding.clone();
        other.thread_id = ThreadId::parse("retained-thread").unwrap();
        assert_eq!(repo.load_path_origins(&other).unwrap(), origins);
        other.provider_thread_id = Some("other-physical".into());
        assert!(repo
            .load_path_origins(&other)
            .unwrap_err()
            .contains("owner"));
        other = binding.clone();
        other.provider_profile_id = LocalProviderProfileId::parse("profile-other").unwrap();
        assert!(repo
            .load_path_origins(&other)
            .unwrap_err()
            .contains("owner"));
        let mut damaged = original.clone();
        damaged[0] = b'[';
        fs::write(&path, damaged).unwrap();
        assert!(repo.load_path_origins(&binding).is_err());
        assert!(repo
            .store_path_origins(PathOriginsOwner::for_binding(&binding).unwrap(), &origins)
            .is_err());
        fs::remove_file(&path).unwrap();
        assert!(repo.load_path_origins(&binding).is_err());
        #[cfg(unix)]
        {
            let outside = temp.path().join("outside-origins.json");
            fs::write(&outside, original).unwrap();
            std::os::unix::fs::symlink(&outside, &path).unwrap();
            assert!(repo.load_path_origins(&binding).is_err());
        }
        let mut serialized = serde_json::to_value(&binding).unwrap();
        serialized
            .as_object_mut()
            .unwrap()
            .remove("path_origins_ref");
        assert!(serde_json::from_value::<LocalConversationBinding>(serialized).is_err());
    }

    #[test]
    fn project_sync_metadata_reset_removes_restored_bindings_only_for_selected_project() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let (binding, origins) = bound_path_origin_fixture(&repo, &temp);
        let mut other_project = project();
        other_project.local_project_id = LocalProjectId::parse("project-b").unwrap();
        other_project.bundle_id = BundleId::parse("ffffffffffffffffffffffffffffffff").unwrap();
        repo.mutate_config(|config| {
            config.projects.push(other_project.clone());
            Ok(())
        })
        .unwrap();
        let claude_profile_id = LocalProviderProfileId::parse("profile-claude").unwrap();
        repo.mutate_bindings(|_, machine| {
            let mut profile = machine.profiles[0].clone();
            profile.profile_id = claude_profile_id.clone();
            profile.provider = ProviderKey::parse("claude").unwrap();
            profile.path = temp.path().join("claude").to_string_lossy().into_owned();
            profile.canonical_path = profile.path.clone();
            machine.profiles.push(profile);
            machine.bindings[0].profile_ids.insert(
                ProviderKey::parse("claude").unwrap(),
                claude_profile_id.clone(),
            );
            let mut replica = machine.bindings[0].clone();
            replica.local_project_id = other_project.local_project_id.clone();
            replica.bundle_id = other_project.bundle_id.clone();
            replica.replica_id = ReplicaId::parse("replica-other").unwrap();
            replica.project_root = temp
                .path()
                .join("other-project")
                .to_string_lossy()
                .into_owned();
            replica.canonical_project_root = replica.project_root.clone();
            machine.bindings.push(replica);
            Ok(())
        })
        .unwrap();
        let mut other_binding = binding.clone();
        other_binding.local_project_id = other_project.local_project_id.clone();
        other_binding.replica_id = ReplicaId::parse("replica-other").unwrap();
        other_binding.thread_id = ThreadId::parse("thread-other").unwrap();
        other_binding.provider_thread_id = Some("other-physical-origins".into());
        let owner =
            crate::session_path_origins::PathOriginsOwner::for_binding(&other_binding).unwrap();
        let mut other_origins = origins.clone();
        let records = other_origins.sources.remove("physical-origins").unwrap();
        other_origins
            .sources
            .insert("other-physical-origins".into(), records);
        other_binding.path_origins_ref =
            Some(repo.store_path_origins(owner, &other_origins).unwrap());
        let mut claude_binding = binding.clone();
        claude_binding.provider_profile_id = claude_profile_id;
        claude_binding.root_tail_layout.as_mut().unwrap().codec =
            mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_V1.into();
        let owner =
            crate::session_path_origins::PathOriginsOwner::for_binding(&claude_binding).unwrap();
        claude_binding.path_origins_ref = Some(repo.store_path_origins(owner, &origins).unwrap());
        repo.mutate_conversation_bindings(|document| {
            document.bindings.extend([
                binding.clone(),
                claude_binding.clone(),
                other_binding.clone(),
            ]);
            Ok(())
        })
        .unwrap();
        let before = repo.load_project_activity_state_snapshot().unwrap();

        // Provider files need not exist: clearing a provider home must not
        // prevent reset from forgetting its restored-session mappings.
        let reset = repo
            .reset_project_sync_metadata(&binding.local_project_id, 0)
            .unwrap();
        assert_eq!(reset.conversation_bindings_removed, 2);
        let after = repo.load_project_activity_state_snapshot().unwrap();
        assert_eq!(
            after.conversation_bindings.bindings,
            vec![other_binding.clone()]
        );
        assert_eq!(
            after.conversation_bindings.revision,
            before.conversation_bindings.revision + 1
        );
        assert_eq!(after.config, before.config);
        assert_eq!(after.bindings, before.bindings);
        assert_eq!(repo.load_path_origins(&binding).unwrap(), origins);
        assert_eq!(repo.load_path_origins(&claude_binding).unwrap(), origins);
        assert_eq!(
            repo.load_path_origins(&other_binding).unwrap(),
            other_origins
        );

        let repeated = repo
            .reset_project_sync_metadata(&binding.local_project_id, 0)
            .unwrap();
        assert_eq!(repeated.conversation_bindings_removed, 0);
        assert_eq!(
            repo.load_conversation_bindings().unwrap(),
            after.conversation_bindings
        );
    }

    #[test]
    fn path_origins_survive_host_identity_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let (binding, origins) = bound_path_origin_fixture(&repo, &temp);
        repo.mutate_conversation_bindings(|document| {
            document.bindings.push(binding.clone());
            Ok(())
        })
        .unwrap();
        fs::remove_file(repo.host_identity_path()).unwrap();
        let next = repo.load_or_create_host_identity().unwrap();
        let current = repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .remove(0);
        assert_ne!(next.host_id, binding.host_id);
        assert_ne!(current.path_origins_ref, binding.path_origins_ref);
        assert_eq!(repo.load_path_origins(&current).unwrap(), origins);
        assert_eq!(repo.load_path_origins(&binding).unwrap(), origins);
    }
}
