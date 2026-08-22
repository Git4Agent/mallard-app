//! Bounded, atomic local persistence.
//!
//! Every file lives below `~/.mallard`. All read-modify-write operations
//! share a process lock and revision check so concurrent Tauri commands
//! cannot silently lose changes.

use mallard_domain::{
    derive_origin_host_token, derive_origin_replica_token, fallback_origin_host_label,
    generated_named_id, BundleId, ConversationBindings, ConversationPushPlan,
    ConversationSyncJournal, HostId, HostIdentity, LocalProjectId, MachineProjectState,
    Materializations, OriginHostToken, OriginReplicaToken, PlanId, ProjectSetupDraft, ReplicaId,
    RestorePlan, SetupDraftId, SetupTransaction, StorageConfig, StorageId, SyncConfig,
    CONVERSATION_BINDINGS_SCHEMA, HOST_IDENTITY_SCHEMA, MACHINE_PROJECT_SCHEMA, MAX_SETUP_DRAFTS,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BINDINGS_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MATERIALIZATIONS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RESTORE_PLAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SETUP_DRAFT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SETUP_TRANSACTION_BYTES: u64 = 8 * 1024 * 1024;
const MAX_HOST_IDENTITY_BYTES: u64 = 16 * 1024;
// Rotation journals embed every bounded host-scoped document. They are stored
// as compact JSON to avoid nested pretty-print expansion; the additional
// headroom covers wrapper fields and bounded default growth.
const MAX_HOST_ROTATION_BYTES: u64 = 192 * 1024 * 1024;
const MAX_CONVERSATION_BINDINGS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONVERSATION_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONVERSATION_PUSH_PLAN_BYTES: u64 = 64 * 1024 * 1024;
const HOST_ROTATION_SCHEMA: u32 = 1;

static PERSISTENCE_LOCK: Mutex<()> = Mutex::new(());

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
    pub journal_operations_removed: u64,
    pub restore_plans_removed: u64,
    pub push_plans_removed: u64,
    pub unreadable_plan_files_retained: u64,
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
    pub conversation_checkpoints_removed: u64,
    pub materializations_removed: u64,
    pub journal_operations_removed: u64,
    pub restore_plans_removed: u64,
    pub push_plans_removed: u64,
    pub unreadable_plan_files_retained: u64,
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

impl<E> ConfigTransactionError<E> {
    pub fn into_persistence(self) -> Option<String> {
        match self {
            Self::Mutation(_) => None,
            Self::Persistence(message) => Some(message),
        }
    }
}

struct PersistenceGuard {
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
    /// The desktop crate's Tauri command layer calls this; `from_app` lived
    /// here before the workspace reorganization moved this module out of the
    /// Tauri crate.
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

    pub fn conversation_sync_journal_path(&self) -> PathBuf {
        self.root.join("conversation_sync_journal.json")
    }

    pub fn materializations_path(&self) -> PathBuf {
        self.root.join("materializations.json")
    }

    pub fn restore_plan_path(&self, plan_id: &PlanId) -> PathBuf {
        self.root
            .join("restore_plans")
            .join(format!("{}.json", plan_id.as_str()))
    }

    pub fn conversation_push_plan_path(&self, plan_id: &PlanId) -> PathBuf {
        self.root
            .join("conversation_push_plans")
            .join(format!("{}.json", plan_id.as_str()))
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

    pub fn update_host_display_name(
        &self,
        expected_host_id: &HostId,
        expected_revision: u64,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        let _operation = self.acquire_host_operation()?;
        let _guard = persistence_guard(&self.root)?;
        let mut identity = self.load_or_create_host_identity_unlocked()?;
        if &identity.host_id != expected_host_id || identity.revision != expected_revision {
            return Err("host identity changed; refresh before saving".to_string());
        }
        identity.local_display_name = local_display_name;
        identity.revision = identity.revision.saturating_add(1);
        identity.updated_at = identity
            .updated_at
            .max(identity.created_at)
            .max(now_secs()?);
        identity.validate()?;
        write_json_atomic(
            &self.root,
            &self.host_identity_path(),
            &identity,
            MAX_HOST_IDENTITY_BYTES,
        )?;
        Ok(identity)
    }

    /// Deliberately rotate the installation identity. Existing project
    /// records get fresh replica IDs and are detached until their provider
    /// scope is revalidated; receipts keep their old host/replica pins.
    pub fn rotate_host_identity(
        &self,
        expected: Option<(&HostId, u64)>,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        let _operation = self.acquire_host_operation()?;
        let _guard = persistence_guard(&self.root)?;
        let current = self.load_or_create_host_identity_unlocked()?;
        if let Some((expected_host_id, expected_revision)) = expected {
            if &current.host_id != expected_host_id || current.revision != expected_revision {
                return Err(
                    "host identity changed; refresh before registering this host".to_string(),
                );
            }
        }
        self.replace_host_identity_unlocked(Some(&current), local_display_name)
    }

    /// Publish a fresh identity and rebase every host-scoped document under
    /// it inside one recoverable rotation journal. With `current` present the
    /// rewrite fails closed on any damaged document; without it (automatic
    /// registration) damaged documents are quarantined and replaced.
    fn replace_host_identity_unlocked(
        &self,
        current: Option<&HostIdentity>,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        let allow_damaged_recovery = current.is_none();
        self.validate_setup_transactions_unlocked(allow_damaged_recovery)?;

        let config = self.load_config_unlocked()?;
        let mut quarantine = HostRotationQuarantine::default();
        let next = self.prepare_host_identity_unlocked(local_display_name)?;
        let machine_result = match current {
            Some(identity) => self.load_bindings_unlocked(&config, identity),
            None => self.load_bindings_document_unlocked(&config),
        };
        let mut machine = match machine_result {
            Ok(machine) => machine,
            Err(_) if allow_damaged_recovery => {
                quarantine.machine_projects = true;
                MachineProjectState::default()
            }
            Err(error) => return Err(error),
        };
        let mut conversations = match current {
            Some(identity) => {
                self.load_conversation_bindings_document_unlocked(&config, &machine, identity)?
            }
            None => match read_json_bounded::<ConversationBindings>(
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
            },
        };
        if allow_damaged_recovery {
            // Salvaged alias rows must not wedge automatic registration.
            let mut seen = BTreeSet::new();
            conversations.origin_aliases.retain(|alias| {
                alias.validate().is_ok()
                    && seen.insert((alias.bundle_id.clone(), alias.origin_host_token.clone()))
            });
            conversations
                .origin_aliases
                .truncate(mallard_domain::MAX_ORIGIN_ALIASES);
        }
        let mut materializations = match self.load_materializations_unlocked(&config) {
            Ok(materializations) => materializations,
            Err(_) if allow_damaged_recovery => {
                quarantine.materializations = true;
                Materializations::default()
            }
            Err(error) => return Err(error),
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
        conversations.bindings.retain_mut(|binding| {
            if binding.validate().is_err() {
                return false;
            }
            let Some(next_replica) = replica_map.get(&binding.replica_id) else {
                return false;
            };
            binding.host_id = next.host_id.clone();
            binding.replica_id = next_replica.clone();
            binding.materialization_state = if binding.provider_thread_id.is_some() {
                mallard_domain::ConversationMaterializationState::Pending
            } else {
                mallard_domain::ConversationMaterializationState::StoredOnly
            };
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = binding.updated_at.max(binding.created_at).max(timestamp);
            true
        });
        conversations.validate(&next, &machine, &config)?;

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

    pub fn resolve_origin_host_label(
        &self,
        bundle_id: &BundleId,
        origin_host_token: &OriginHostToken,
    ) -> Result<(String, bool), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let is_this_host =
            derive_origin_host_token(&identity.host_id, bundle_id)? == *origin_host_token;
        let display_name = if is_this_host {
            "This host".to_string()
        } else {
            self.load_conversation_bindings_for_aliases_unlocked(&identity)?
                .origin_aliases
                .into_iter()
                .find(|alias| {
                    alias.bundle_id == *bundle_id && alias.origin_host_token == *origin_host_token
                })
                .map(|alias| alias.display_name)
                .unwrap_or_else(|| fallback_origin_host_label(origin_host_token))
        };
        Ok((display_name, is_this_host))
    }

    pub fn load_conversation_bindings(&self) -> Result<ConversationBindings, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        self.load_conversation_bindings_document_unlocked(&config, &machine, &identity)
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
        bindings.validate(&identity, &machine, &config)?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            &bindings,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )?;
        Ok(result)
    }

    /// Install one journaled whole-document bindings after-state exactly as
    /// recorded: validated, atomically written, and never re-revisioned. The
    /// caller proves the current document equals the journaled before-state.
    pub fn install_conversation_bindings_snapshot(
        &self,
        document: ConversationBindings,
    ) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        document.validate(&identity, &machine, &config)?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            &document,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )
    }

    /// Install one journaled whole-document `SyncConfig` after-state exactly
    /// as recorded. Metadata-only Pull actions and journaled operations both
    /// commit through this single atomic replacement.
    pub fn install_config_snapshot(&self, document: SyncConfig) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        document.validate()?;
        write_json_atomic(&self.root, &self.config_path(), &document, MAX_CONFIG_BYTES)
    }

    pub fn load_conversation_sync_journal(&self) -> Result<ConversationSyncJournal, String> {
        let _guard = persistence_guard(&self.root)?;
        let journal = read_json_bounded(
            &self.root,
            &self.conversation_sync_journal_path(),
            MAX_CONVERSATION_JOURNAL_BYTES,
        )?
        .unwrap_or_else(ConversationSyncJournal::empty);
        journal.validate()?;
        Ok(journal)
    }

    pub fn mutate_conversation_sync_journal<T>(
        &self,
        expected_revision: u64,
        mutate: impl FnOnce(&mut ConversationSyncJournal) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let mut journal: ConversationSyncJournal = read_json_bounded(
            &self.root,
            &self.conversation_sync_journal_path(),
            MAX_CONVERSATION_JOURNAL_BYTES,
        )?
        .unwrap_or_else(ConversationSyncJournal::empty);
        journal.validate()?;
        if journal.revision != expected_revision {
            return Err("conversation sync journal changed after review".to_string());
        }
        let result = mutate(&mut journal)?;
        journal.revision = journal.revision.saturating_add(1);
        journal.validate()?;
        write_json_atomic(
            &self.root,
            &self.conversation_sync_journal_path(),
            &journal,
            MAX_CONVERSATION_JOURNAL_BYTES,
        )?;
        Ok(result)
    }

    pub fn mutate_origin_aliases<T>(
        &self,
        mutate: impl FnOnce(&mut Vec<mallard_domain::OriginHostAlias>) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let mut document = self.load_conversation_bindings_for_aliases_unlocked(&identity)?;
        let result = mutate(&mut document.origin_aliases)?;
        document.revision = document.revision.saturating_add(1);
        self.validate_origin_aliases_unlocked(&document)?;
        write_json_atomic(
            &self.root,
            &self.conversation_bindings_path(),
            &document,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )?;
        Ok(result)
    }

    pub fn load_config(&self) -> Result<SyncConfig, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_config_unlocked()
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

    /// Forget every derived sync decision for one configured project while
    /// retaining its setup and every user-owned byte. The reset deliberately
    /// abandons matching recovery operations: their provider/cloud effects
    /// are left in place and will be rediscovered, never rolled back or
    /// deleted under this new approval.
    ///
    /// All prospective documents are validated before the first write. Each
    /// write is an atomic replacement and each intermediate state is valid,
    /// so a process crash is repaired by rerunning this idempotent reset; it
    /// does not need a second recovery journal.
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
        let project_replica = machine
            .for_project(local_project_id)
            .map(|binding| (binding.host_id.clone(), binding.replica_id.clone()));
        let mut conversation_bindings =
            self.load_conversation_bindings_document_unlocked(&config, &machine, &identity)?;
        let mut materializations = self.load_materializations_unlocked(&config)?;
        let mut journal: ConversationSyncJournal = read_json_bounded(
            &self.root,
            &self.conversation_sync_journal_path(),
            MAX_CONVERSATION_JOURNAL_BYTES,
        )?
        .unwrap_or_else(ConversationSyncJournal::empty);
        journal.validate()?;

        let (restore_plans, restore_warnings) = self.list_documents::<RestorePlan>(
            &self.root.join("restore_plans"),
            MAX_RESTORE_PLAN_BYTES,
        )?;
        let (push_plans, push_warnings) = self.list_documents::<ConversationPushPlan>(
            &self.root.join("conversation_push_plans"),
            MAX_CONVERSATION_PUSH_PLAN_BYTES,
        )?;
        let restore_plan_ids = restore_plans
            .into_iter()
            .filter(|plan| {
                plan.conversation_actions
                    .iter()
                    .any(|action| &action.pins.context.local_project_id == local_project_id)
                    || project_replica
                        .as_ref()
                        .is_some_and(|(host_id, replica_id)| {
                            &plan.host_id == host_id && &plan.replica_id == replica_id
                        })
            })
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();
        let push_plan_ids = push_plans
            .into_iter()
            .filter(|plan| &plan.local_project_id == local_project_id)
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();

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

        let journal_count = journal.operations.len();
        journal
            .operations
            .retain(|operation| &operation.local_project_id != local_project_id);
        let journal_operations_removed = (journal_count - journal.operations.len()) as u64;

        if config_changed {
            config.revision = config.revision.saturating_add(1);
        }
        if conversation_bindings_removed > 0 {
            conversation_bindings.revision = conversation_bindings.revision.saturating_add(1);
        }
        if materializations_removed > 0 {
            materializations.revision = materializations.revision.saturating_add(1);
        }
        if journal_operations_removed > 0 {
            journal.revision = journal.revision.saturating_add(1);
        }

        config.validate()?;
        machine.validate(&config)?;
        conversation_bindings.validate(&identity, &machine, &config)?;
        materializations.validate(&config)?;
        journal.validate()?;

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
        if journal_operations_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.conversation_sync_journal_path(),
                &journal,
                MAX_CONVERSATION_JOURNAL_BYTES,
            )?;
        }

        for plan_id in &restore_plan_ids {
            let path = self.restore_plan_path(plan_id);
            ensure_no_symlinks(&self.root, &path)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("remove restore plan '{}': {}", plan_id, error)),
            }
        }
        for plan_id in &push_plan_ids {
            let path = self.conversation_push_plan_path(plan_id);
            ensure_no_symlinks(&self.root, &path)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "remove conversation push plan '{}': {}",
                        plan_id, error
                    ))
                }
            }
        }

        Ok(ProjectSyncMetadataResetResult {
            local_project_id: local_project_id.clone(),
            config_revision: config.revision,
            links_reset,
            scope_bases_removed,
            conversation_checkpoints_removed,
            conversation_bindings_removed,
            materializations_removed,
            journal_operations_removed,
            restore_plans_removed: restore_plan_ids.len() as u64,
            push_plans_removed: push_plan_ids.len() as u64,
            unreadable_plan_files_retained: restore_warnings
                .len()
                .saturating_add(push_warnings.len())
                as u64,
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
        let mut journal: ConversationSyncJournal = read_json_bounded(
            &self.root,
            &self.conversation_sync_journal_path(),
            MAX_CONVERSATION_JOURNAL_BYTES,
        )?
        .unwrap_or_else(ConversationSyncJournal::empty);
        journal.validate()?;
        let (restore_plans, restore_warnings) = self.list_documents::<RestorePlan>(
            &self.root.join("restore_plans"),
            MAX_RESTORE_PLAN_BYTES,
        )?;
        let (push_plans, push_warnings) = self.list_documents::<ConversationPushPlan>(
            &self.root.join("conversation_push_plans"),
            MAX_CONVERSATION_PUSH_PLAN_BYTES,
        )?;
        let restore_plan_ids = restore_plans
            .into_iter()
            .filter(|plan| &plan.storage_id == storage_id)
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();
        let push_plan_ids = push_plans
            .into_iter()
            .filter(|plan| &plan.storage_id == storage_id)
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();

        let mut links_reset = 0_u64;
        let mut scope_bases_removed = 0_u64;
        let mut conversation_checkpoints_removed = 0_u64;
        let mut config_changed = false;
        for link in config
            .links
            .iter_mut()
            .filter(|link| &link.storage_id == storage_id)
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

        let materialization_count = materializations.records.len();
        materializations
            .records
            .retain(|record| &record.storage_id != storage_id);
        let materializations_removed =
            (materialization_count - materializations.records.len()) as u64;

        let journal_count = journal.operations.len();
        journal
            .operations
            .retain(|operation| &operation.cloud.storage_id != storage_id);
        let journal_operations_removed = (journal_count - journal.operations.len()) as u64;

        if config_changed {
            config.revision = config.revision.saturating_add(1);
        }
        if materializations_removed > 0 {
            materializations.revision = materializations.revision.saturating_add(1);
        }
        if journal_operations_removed > 0 {
            journal.revision = journal.revision.saturating_add(1);
        }
        config.validate()?;
        materializations.validate(&config)?;
        journal.validate()?;

        let remote_result = clear_remote(&storage)?;

        if materializations_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.materializations_path(),
                &materializations,
                MAX_MATERIALIZATIONS_BYTES,
            )?;
        }
        if journal_operations_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.conversation_sync_journal_path(),
                &journal,
                MAX_CONVERSATION_JOURNAL_BYTES,
            )?;
        }
        for plan_id in &restore_plan_ids {
            let path = self.restore_plan_path(plan_id);
            ensure_no_symlinks(&self.root, &path)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("remove restore plan '{}': {}", plan_id, error)),
            }
        }
        for plan_id in &push_plan_ids {
            let path = self.conversation_push_plan_path(plan_id);
            ensure_no_symlinks(&self.root, &path)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "remove conversation push plan '{}': {}",
                        plan_id, error
                    ))
                }
            }
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
                conversation_checkpoints_removed,
                materializations_removed,
                journal_operations_removed,
                restore_plans_removed: restore_plan_ids.len() as u64,
                push_plans_removed: push_plan_ids.len() as u64,
                unreadable_plan_files_retained: restore_warnings
                    .len()
                    .saturating_add(push_warnings.len())
                    as u64,
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

    /// Restore plans are immutable, revision-pinned documents.  Reusing a
    /// plan ID is always an error instead of replacing an approval surface.
    pub fn save_restore_plan(&self, plan: &RestorePlan) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        plan.validate()?;
        let path = self.restore_plan_path(&plan.plan_id);
        self.ensure_directory(path.parent().ok_or("restore plan has no parent")?)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!("restore plan '{}' already exists", plan.plan_id));
        }
        write_json_atomic(&self.root, &path, plan, MAX_RESTORE_PLAN_BYTES)
    }

    pub fn load_restore_plan(&self, plan_id: &PlanId) -> Result<RestorePlan, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.restore_plan_path(plan_id);
        let plan: RestorePlan = read_json_bounded(&self.root, &path, MAX_RESTORE_PLAN_BYTES)?
            .ok_or_else(|| format!("restore plan '{}' does not exist", plan_id))?;
        plan.validate()?;
        Ok(plan)
    }

    /// Reviewed Push plans are immutable and host-pinned, just like Pull
    /// plans. Keeping them in a separate directory prevents one plan type
    /// from being read as the other.
    pub fn save_conversation_push_plan(&self, plan: &ConversationPushPlan) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        plan.validate()?;
        let path = self.conversation_push_plan_path(&plan.plan_id);
        self.ensure_directory(
            path.parent()
                .ok_or("conversation push plan has no parent")?,
        )?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!(
                "conversation push plan '{}' already exists",
                plan.plan_id
            ));
        }
        write_json_atomic(&self.root, &path, plan, MAX_CONVERSATION_PUSH_PLAN_BYTES)
    }

    pub fn load_conversation_push_plan(
        &self,
        plan_id: &PlanId,
    ) -> Result<ConversationPushPlan, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.conversation_push_plan_path(plan_id);
        let plan: ConversationPushPlan =
            read_json_bounded(&self.root, &path, MAX_CONVERSATION_PUSH_PLAN_BYTES)?
                .ok_or_else(|| format!("conversation push plan '{}' does not exist", plan_id))?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn discard_conversation_push_plan(&self, plan_id: &PlanId) -> Result<bool, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.conversation_push_plan_path(plan_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!(
                "remove conversation push plan '{}': {}",
                plan_id, error
            )),
        }
    }

    fn load_config_unlocked(&self) -> Result<SyncConfig, String> {
        let config: SyncConfig =
            read_json_bounded(&self.root, &self.config_path(), MAX_CONFIG_BYTES)?
                .unwrap_or_default();
        config.validate()?;
        Ok(config)
    }

    fn load_or_create_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        self.recover_host_rotation_unlocked()?;
        match self.load_host_identity_document_classified_unlocked() {
            Ok(Some(identity)) => {
                sync_directory(&self.root)?;
                Ok(identity)
            }
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
            return self.create_host_identity_unlocked(None);
        }
        self.replace_host_identity_unlocked(None, None)
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

    fn create_host_identity_unlocked(
        &self,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        let identity = self.prepare_host_identity_unlocked(local_display_name)?;
        write_json_atomic(
            &self.root,
            &self.host_identity_path(),
            &identity,
            MAX_HOST_IDENTITY_BYTES,
        )?;
        Ok(identity)
    }

    fn prepare_host_identity_unlocked(
        &self,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        let timestamp = now_secs()?;
        let identity = HostIdentity {
            schema: HOST_IDENTITY_SCHEMA,
            revision: 0,
            host_id: HostId::generate()?,
            local_display_name,
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

    fn validate_setup_transactions_unlocked(
        &self,
        allow_damaged_recovery: bool,
    ) -> Result<(), String> {
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
                Err(_) if allow_damaged_recovery => quarantines.push(path),
                Err(error) => {
                    return Err(format!(
                        "setup transaction is unreadable or invalid; register as a new host to quarantine it ({})",
                        error
                    ))
                }
            }
        }
        // Verified rotation validates the complete directory before changing
        // any setup journal. Imported recovery quarantines only the entries
        // it could not validate.
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
        bindings.validate(identity, machine, config)?;
        Ok(bindings)
    }

    fn load_conversation_bindings_for_aliases_unlocked(
        &self,
        identity: &HostIdentity,
    ) -> Result<ConversationBindings, String> {
        let document = self
            .read_conversation_bindings_unlocked()?
            .unwrap_or_else(|| ConversationBindings::empty(identity.host_id.clone()));
        self.validate_origin_aliases_unlocked(&document)?;
        if document.host_id != identity.host_id {
            return Err("origin aliases belong to a different host document".to_string());
        }
        Ok(document)
    }

    fn validate_origin_aliases_unlocked(
        &self,
        document: &ConversationBindings,
    ) -> Result<(), String> {
        if document.schema != CONVERSATION_BINDINGS_SCHEMA
            || document.bindings.len() > mallard_domain::MAX_CONVERSATION_BINDINGS
            || document.origin_aliases.len() > mallard_domain::MAX_ORIGIN_ALIASES
        {
            return Err("invalid conversation bindings document".to_string());
        }
        let mut aliases = HashSet::new();
        for alias in &document.origin_aliases {
            alias.validate()?;
            if !aliases.insert((alias.bundle_id.clone(), alias.origin_host_token.clone())) {
                return Err("duplicate origin host alias".to_string());
            }
        }
        Ok(())
    }

    /// Explicit host registration must remain an escape hatch for copied or
    /// partially restored state. Preserve a fully valid conversation document;
    /// otherwise quarantine its host-bound associations while salvaging only
    /// independently valid, local presentation aliases.
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
        ensure_no_symlinks(&self.root, &self.root)
    }

    fn ensure_directory(&self, directory: &Path) -> Result<(), String> {
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

fn persistence_guard(root: &Path) -> Result<PersistenceGuard, String> {
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
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure project-sync persistence lock: {}", error))?;
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

fn ensure_no_symlinks(root: &Path, destination: &Path) -> Result<(), String> {
    let relative = destination.strip_prefix(root).map_err(|_| {
        format!(
            "Mallard state path '{}' escapes '{}'",
            destination.display(),
            root.display()
        )
    })?;
    let mut current = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Mallard state path traverses symlink '{}'",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect '{}': {}", current.display(), error)),
        }
    }
    Ok(())
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
    ensure_no_symlinks(root, path).map_err(BoundedJsonReadError::Io)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BoundedJsonReadError::Io(format!(
                "inspect '{}': {}",
                path.display(),
                error
            )))
        }
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

pub fn write_json_atomic<T: Serialize>(
    root: &Path,
    path: &Path,
    value: &T,
    max_bytes: u64,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    write_bytes_atomic(root, path, &bytes, max_bytes)
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

fn write_bytes_atomic(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    max_bytes: u64,
) -> Result<(), String> {
    ensure_no_symlinks(root, path)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("'{}' has no parent", path.display()))?;
    if !parent.exists() {
        return Err(format!(
            "parent directory '{}' does not exist",
            parent.display()
        ));
    }
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
    use mallard_domain::{
        generated_named_id, BindingState, BundleId, BundleRecipe, ConversationMaterializationState,
        LocalConversationBinding, LocalProjectRegistration, LocalProviderProfileId,
        MaterializationId, MaterializationRecord, MaterializationStatus, OriginHostAlias, PlanId,
        PreservedProvenance, ProjectBinding, ProjectStorageLink, ProviderKey, ProviderProfile,
        RepositoryBase, RootTailLayout, ScopeBase, StorageConfig, StorageId, StorageKind,
        SyncScopeKey, TailPartitionKind, ThreadId, LOCAL_SCHEMA, PORTABLE_TAIL_CODEC_V1,
    };
    use std::collections::BTreeMap;

    fn codex_key() -> ProviderKey {
        ProviderKey::parse("codex").unwrap()
    }

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path()).unwrap()
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
            config.storages.push(StorageConfig {
                id: storage_id.clone(),
                name: "Storage A".to_string(),
                kind: StorageKind::Local,
                bucket: String::new(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                account_id: String::new(),
                s3_endpoint: String::new(),
                region: String::new(),
                local_dir: storage_root.to_string_lossy().into_owned(),
                supports_conditional_writes: None,
            });
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
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let replica_id = install_bound_project(&repo, &temp, &identity);
        let local_project_id = LocalProjectId::parse("project-a").unwrap();
        let storage_id = StorageId::parse("storage-a").unwrap();
        repo.mutate_config(|config| {
            config.storages.push(StorageConfig {
                id: storage_id.clone(),
                name: "Storage A".to_string(),
                kind: StorageKind::Local,
                bucket: String::new(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                account_id: String::new(),
                s3_endpoint: String::new(),
                region: String::new(),
                local_dir: temp.path().join("storage").to_string_lossy().into_owned(),
                supports_conditional_writes: None,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_conversation_bindings(|bindings| {
            bindings.bindings.push(LocalConversationBinding {
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: local_project_id.clone(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: ThreadId::parse("thread-preserved").unwrap(),
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
        let (_, result) = repo
            .clear_storage_data(&storage_id, expected_config_revision, |_| {
                Ok::<_, String>(())
            })
            .unwrap();

        assert_eq!(result.materializations_removed, 1);
        assert!(repo.load_materializations().unwrap().records.is_empty());
        let bindings = repo.load_conversation_bindings().unwrap();
        assert_eq!(bindings.bindings.len(), 1);
        assert_eq!(
            bindings.bindings[0].preserved,
            Some(PreservedProvenance {
                preserved_from: mallard_domain::CloudThreadId::parse("cloudthread-origin").unwrap(),
                ordinal: 3,
            })
        );
    }

    #[test]
    fn project_sync_metadata_reset_rejects_a_stale_project_revision_before_writes() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        install_bound_project(&repo, &temp, &identity);
        let before = repo.load_config().unwrap();

        let error = repo
            .reset_project_sync_metadata(
                &LocalProjectId::parse("project-a").unwrap(),
                before.projects[0].revision.saturating_add(1),
            )
            .unwrap_err();

        assert!(error.contains("project changed"));
        assert_eq!(repo.load_config().unwrap(), before);
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
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_conversation_bindings(|conversations| {
            conversations.bindings.push(LocalConversationBinding {
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

    #[test]
    fn restore_plan_ids_get_private_namespaced_paths() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let plan = PlanId::parse(generated_named_id("plan").unwrap()).unwrap();
        let path = repo.restore_plan_path(&plan);
        assert!(path.starts_with(temp.path().join(".mallard/restore_plans")));
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!("{}.json", plan)
        );
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

        assert_eq!(
            repo.resolve_origin_host_label(&bundle_a, &a.0).unwrap(),
            ("This host".to_string(), true)
        );
        let remote_token = second_repo
            .derive_origin_tokens(&bundle_a, &replica)
            .unwrap()
            .0;
        assert_eq!(
            repo.resolve_origin_host_label(&bundle_a, &remote_token)
                .unwrap(),
            (fallback_origin_host_label(&remote_token), false)
        );
        repo.mutate_origin_aliases(|aliases| {
            aliases.push(OriginHostAlias {
                bundle_id: bundle_a.clone(),
                origin_host_token: remote_token.clone(),
                display_name: "Remote laptop".to_string(),
                revision: 0,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(
            repo.resolve_origin_host_label(&bundle_a, &remote_token)
                .unwrap(),
            ("Remote laptop".to_string(), false)
        );
    }

    #[test]
    fn display_name_updates_are_revision_guarded_and_clock_monotonic() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let mut identity = repo.load_or_create_host_identity().unwrap();
        identity.revision = 7;
        identity.created_at = u64::MAX - 1;
        identity.updated_at = u64::MAX - 1;
        write_json_atomic(
            repo.root(),
            &repo.host_identity_path(),
            &identity,
            MAX_HOST_IDENTITY_BYTES,
        )
        .unwrap();

        let updated = repo
            .update_host_display_name(
                &identity.host_id,
                identity.revision,
                Some("Work laptop".to_string()),
            )
            .unwrap();
        assert_eq!(updated.revision, 8);
        assert_eq!(updated.updated_at, u64::MAX - 1);
        assert!(repo
            .update_host_display_name(&identity.host_id, identity.revision, None)
            .unwrap_err()
            .contains("refresh before saving"));
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
    fn verified_rotation_preserves_invalid_setup_journals_for_explicit_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let path = repo.root().join("setup_transactions").join("broken.json");
        repo.ensure_directory(path.parent().unwrap()).unwrap();
        let malformed = b"{\"schema\":2}\n";
        fs::write(&path, malformed).unwrap();

        let error = repo
            .rotate_host_identity(Some((&previous.host_id, previous.revision)), None)
            .unwrap_err();
        assert!(error.contains("unreadable or invalid"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), malformed);
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(previous.clone())
        );
        assert!(!repo.host_rotation_path().exists());
        assert!(!repo.root().join("host_recovery_quarantine").exists());

        // Automatic registration is damage tolerant: it quarantines the
        // malformed setup journal instead of failing closed.
        repo.mutate_conversation_bindings(|_| Ok(())).unwrap();
        fs::remove_file(repo.host_identity_path()).unwrap();
        let registered = repo.load_or_create_host_identity().unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert!(!path.exists());
        assert!(fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("setup-transaction-")));
    }

    #[test]
    fn verified_rotation_fails_closed_on_damaged_host_scoped_state() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let damaged = b"{not valid machine state";
        fs::write(repo.machine_projects_path(), damaged).unwrap();

        let error = repo
            .rotate_host_identity(Some((&identity.host_id, identity.revision)), None)
            .unwrap_err();
        assert!(error.contains("parse"), "{error}");
        assert_eq!(fs::read(repo.machine_projects_path()).unwrap(), damaged);
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(identity)
        );
        assert!(!repo.host_rotation_path().exists());
    }

    #[test]
    fn verified_rotation_rejects_a_structurally_valid_foreign_host_binding() {
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

        let error = repo
            .rotate_host_identity(Some((&identity.host_id, identity.revision)), None)
            .unwrap_err();
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
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
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
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
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
        fs::write(&setup, b"{also-not-json").unwrap();
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

        // Explicit rotation stays strict: an invalid display name fails
        // before any load, and damaged documents keep it failing closed.
        let error = repo
            .rotate_host_identity(
                Some((&previous.host_id, previous.revision)),
                Some("\n".to_string()),
            )
            .unwrap_err();
        assert!(error.contains("invalid local host display name"));
        assert_eq!(
            fs::read(repo.machine_projects_path()).unwrap(),
            b"{broken-machine"
        );
        assert_eq!(
            fs::read(repo.materializations_path()).unwrap(),
            b"{broken-materializations"
        );
        assert!(!repo.host_rotation_path().exists());

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
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
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
        // Resume the state left by a failed destination-directory sync: both
        // names refer to the same preserved inode, so the live name may now
        // be retired without treating the destination as a collision.
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
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
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
}
