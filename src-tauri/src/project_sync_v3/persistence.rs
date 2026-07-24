//! Bounded, atomic, schema-3 local persistence.
//!
//! Every file lives below `~/.mallard`; schema-2 configuration, baselines,
//! backups, and machine records are neither read nor overwritten. All
//! read-modify-write operations share a process lock and revision check so
//! concurrent Tauri commands cannot silently lose changes.

use super::domain::{
    derive_origin_host_token, derive_origin_key_verifier, derive_origin_replica_token,
    fallback_origin_host_label, generated_named_id, BundleId, ConversationBindings,
    ConversationPushPlan, ConversationSyncJournal, DependencyApplications, DependencyPlan, HostId,
    HostIdentity, LocalProjectId, MachineProjectState, Materializations, OriginHostToken,
    OriginKey, OriginKeyRef, OriginReplicaToken, PlanId, ProjectSetupDraft, ReplicaId, RestorePlan,
    SetupDraftId, SetupTransaction, SyncConfigV3, CONVERSATION_BINDINGS_SCHEMA_V2,
    HOST_IDENTITY_SCHEMA_V1, MACHINE_PROJECT_SCHEMA_V1, MACHINE_PROJECT_SCHEMA_V2,
    MAX_SETUP_DRAFTS, SETUP_TRANSACTION_SCHEMA_V1, SETUP_TRANSACTION_SCHEMA_V2,
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
const MAX_DEPENDENCY_PLAN_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEPENDENCY_APPLICATIONS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SETUP_DRAFT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SETUP_TRANSACTION_BYTES: u64 = 8 * 1024 * 1024;
const MAX_HOST_IDENTITY_BYTES: u64 = 16 * 1024;
// Rotation journals embed every bounded host-scoped document. They are stored
// as compact JSON to avoid nested pretty-print expansion; the additional
// headroom covers wrapper fields and bounded migration/default growth.
const MAX_HOST_ROTATION_BYTES: u64 = 192 * 1024 * 1024;
const MAX_CONVERSATION_BINDINGS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONVERSATION_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONVERSATION_PUSH_PLAN_BYTES: u64 = 64 * 1024 * 1024;
const ORIGIN_KEY_BYTES: u64 = 32;
#[cfg(windows)]
const MAX_PROTECTED_ORIGIN_KEY_BYTES: u64 = 16 * 1024;
#[cfg(all(target_os = "macos", not(test)))]
const MACOS_KEYCHAIN_MARKER: &[u8] = b"mallard-keychain-v1";
#[cfg(all(target_os = "macos", not(test)))]
const MACOS_KEYCHAIN_SERVICE: &str = "com.hequ.agent-sync.mallard-origin-key-v1";
const MAX_HOST_SCOPED_DOCUMENTS: usize = 100_000;
const MISSING_ORIGIN_KEY_REF: &str = "missing-origin-key";
const UNKNOWN_ORIGIN_KEY_REF: &str = "unknown-origin-state";

static PERSISTENCE_LOCK: Mutex<()> = Mutex::new(());

struct PersistenceGuard {
    _process: std::sync::MutexGuard<'static, ()>,
    _file: fs::File,
}

struct PreparedOriginKeyGuard {
    path: PathBuf,
    armed: bool,
}

impl PreparedOriginKeyGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PreparedOriginKeyGuard {
    fn drop(&mut self) {
        if self.armed {
            #[cfg(all(target_os = "macos", not(test)))]
            {
                let Some(account) = self.path.file_stem().and_then(|stem| stem.to_str()) else {
                    return;
                };
                // The account is derivable from the protected path even when
                // a marker was truncated or replaced. Preserve that path when
                // Keychain cleanup cannot be confirmed so explicit recovery
                // never leaves an untracked privacy-sensitive item.
                if delete_macos_keychain_origin_key(account).is_err() {
                    return;
                }
            }
            let _ = fs::remove_file(&self.path);
            if let Some(parent) = self.path.parent() {
                let _ = sync_directory(parent);
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct HostRotationQuarantine {
    #[serde(default)]
    machine_projects: bool,
    #[serde(default)]
    conversation_bindings: bool,
    #[serde(default)]
    materializations: bool,
    #[serde(default)]
    dependency_applications: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct HostRotationTransaction {
    schema: u32,
    previous_host_id: HostId,
    previous_identity_revision: u64,
    previous_origin_key_ref: OriginKeyRef,
    next_identity: HostIdentity,
    machine_projects: MachineProjectState,
    conversation_bindings: ConversationBindings,
    materializations: Materializations,
    #[serde(default)]
    quarantine: HostRotationQuarantine,
    created_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostRotationMode {
    VerifiedRotation,
    ImportedStateRecovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostRotationRecoveryState {
    Ready,
    CleanupPending,
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

/// Held across host-sensitive workflows (restore/dependency apply, setup,
/// publication, and rotation). The OS lock coordinates separate Mallard
/// processes; dropping the file releases it on every supported platform.
#[derive(Debug)]
pub(crate) struct HostOperationGuard {
    _file: fs::File,
}

#[derive(Clone, Debug)]
pub struct V3Repository {
    root: PathBuf,
}

impl V3Repository {
    /// Resolve the machine-global metadata directory for any Tauri runtime.
    pub fn from_app<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> Result<Self, String> {
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

    pub(crate) fn acquire_host_operation(&self) -> Result<HostOperationGuard, String> {
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

    fn origin_keys_dir(&self) -> PathBuf {
        self.root.join("origin_keys")
    }

    fn origin_key_path(&self, key_ref: &OriginKeyRef) -> PathBuf {
        self.origin_keys_dir()
            .join(format!("{}.key", key_ref.as_str()))
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

    pub fn dependency_applications_path(&self) -> PathBuf {
        self.root.join("dependency_applications.json")
    }

    pub fn restore_plan_path(&self, plan_id: &PlanId) -> PathBuf {
        self.root
            .join("restore_plans")
            .join(format!("{}.json", plan_id.as_str()))
    }

    pub fn dependency_plan_path(&self, plan_id: &PlanId) -> PathBuf {
        self.root
            .join("dependency_plans")
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

    /// Load the installation identity, creating it and its protected fallback
    /// key on first use. An existing identity whose key is unavailable is not
    /// silently replaced: callers must explicitly register a new host.
    pub fn load_or_create_host_identity(&self) -> Result<HostIdentity, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()
    }

    /// Inspect identity availability without exposing or regenerating a
    /// missing origin key. This is the recovery surface for copied/imported
    /// state that needs an explicit "Register as new host" decision.
    pub fn inspect_host_identity(&self) -> Result<(HostIdentity, bool), String> {
        let _guard = persistence_guard(&self.root)?;
        // A copied state tree may contain a pending rotation but omit the
        // protected next key. Keep the status surface reachable so the user
        // can explicitly supersede that unrecoverable journal by registering
        // as a new host.
        let rotation_recoverable = self.recover_host_rotation_unlocked().is_ok();
        let pending_result: Result<Option<HostRotationTransaction>, BoundedJsonReadError> =
            if rotation_recoverable {
                Ok(None)
            } else {
                self.load_host_rotation_transaction_classified_unlocked()
            };
        if let Err(BoundedJsonReadError::Io(error)) = &pending_result {
            return Err(error.clone());
        }
        if pending_result
            .as_ref()
            .is_ok_and(|transaction| transaction.is_some())
        {
            // Propagate durability failures separately from decode failures;
            // only the latter may enter explicit quarantine recovery.
            sync_directory(&self.root)?;
        }
        let pending_rotation = pending_result
            .as_ref()
            .ok()
            .and_then(|value| value.as_ref());
        let pending_rotation_anchored = pending_rotation
            .map(|transaction| self.rotation_is_anchored_without_identity_unlocked(transaction))
            .transpose()
            .ok()
            .flatten()
            .unwrap_or(false);
        let (identity_document, identity_was_invalid) =
            match self.load_host_identity_document_classified_unlocked() {
                Ok(identity) => (identity, false),
                Err(BoundedJsonReadError::Corrupt(_)) => {
                    self.quarantine_file_unlocked(&self.host_identity_path(), "host-identity")?;
                    (None, true)
                }
                Err(BoundedJsonReadError::Io(error)) => return Err(error),
            };
        if identity_document.is_some() {
            // A valid identity whose directory sync fails is not corrupt and
            // must never be quarantined as though its bytes were untrusted.
            sync_directory(&self.root)?;
        }
        let identity = match identity_document {
            Some(identity) => identity,
            None if pending_rotation_anchored => {
                self.synthetic_previous_identity(pending_rotation.unwrap())?
            }
            None => match self.infer_missing_host_identity_unlocked() {
                Ok(Some(identity)) => identity,
                Ok(None)
                    if rotation_recoverable
                        && !identity_was_invalid
                        && self.host_identity_storage_is_clean_unlocked()? =>
                {
                    self.create_host_identity_unlocked(None)?
                }
                // Ambiguous or malformed retained state cannot safely be
                // assigned to any historical host. Persist a recovery nonce
                // so the user still has a revision-guarded path to register a
                // genuinely new host without guessing at old ownership.
                Ok(None) | Err(_) => self.create_unknown_identity_marker_unlocked()?,
            },
        };
        let mut identity = identity;
        let key_available = if rotation_recoverable
            && pending_rotation.is_none()
            && !self.is_unknown_identity_marker(&identity)
        {
            match self.load_origin_key_unlocked(&identity) {
                Ok(key) => {
                    self.ensure_origin_key_verifier_unlocked(&mut identity, &key)?;
                    true
                }
                Err(_) => false,
            }
        } else {
            false
        };
        // Cleanup is safe only after a live identity and key have both been
        // resolved. Missing, damaged, or imported state may still need an
        // otherwise well-formed key for explicit recovery.
        if key_available {
            self.collect_orphaned_origin_keys_unlocked()?;
        }
        Ok((identity, key_available))
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

    /// Explicitly establish a new installation identity. Existing project
    /// records get fresh replica IDs and are detached until their provider
    /// scope is revalidated; receipts keep their old host/replica pins.
    pub fn register_as_new_host(
        &self,
        expected: Option<(&HostId, u64)>,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        self.replace_host_identity(
            expected,
            local_display_name,
            HostRotationMode::ImportedStateRecovery,
        )
    }

    /// Deliberately rotate a healthy installation identity. Unlike imported
    /// state recovery, routine rotation fails closed on any damaged local
    /// document instead of substituting empty state.
    pub fn rotate_host_identity(
        &self,
        expected: Option<(&HostId, u64)>,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        self.replace_host_identity(
            expected,
            local_display_name,
            HostRotationMode::VerifiedRotation,
        )
    }

    fn replace_host_identity(
        &self,
        expected: Option<(&HostId, u64)>,
        local_display_name: Option<String>,
        mode: HostRotationMode,
    ) -> Result<HostIdentity, String> {
        let _operation = self.acquire_host_operation()?;
        let _guard = persistence_guard(&self.root)?;
        let allow_damaged_recovery = mode == HostRotationMode::ImportedStateRecovery;
        let recovery_result = self.recover_host_rotation_unlocked();
        if matches!(
            recovery_result,
            Ok(HostRotationRecoveryState::CleanupPending)
        ) {
            return Err(
                "previous host identity cleanup is still pending; retry after protected key storage becomes available"
                    .to_string(),
            );
        }
        let recovery_error = recovery_result.err();
        let pending_result = if recovery_error.is_some() {
            self.load_host_rotation_transaction_classified_unlocked()
        } else {
            Ok(None)
        };
        if let Err(BoundedJsonReadError::Io(error)) = &pending_result {
            return Err(error.clone());
        }
        if pending_result
            .as_ref()
            .is_ok_and(|transaction| transaction.is_some())
        {
            sync_directory(&self.root)?;
        }
        let pending_unreadable = matches!(&pending_result, Err(BoundedJsonReadError::Corrupt(_)));
        let pending_rotation = pending_result.unwrap_or(None);
        let pending_rotation_anchored = pending_rotation
            .as_ref()
            .map(|transaction| self.rotation_is_anchored_without_identity_unlocked(transaction))
            .transpose()?
            .unwrap_or(false);
        let current_document = self.load_host_identity_document_unlocked()?;
        if current_document.is_some() {
            sync_directory(&self.root)?;
        }
        let current = match current_document {
            Some(identity) => identity,
            None if pending_rotation_anchored => {
                self.synthetic_previous_identity(pending_rotation.as_ref().unwrap())?
            }
            None => match self.infer_missing_host_identity_unlocked()? {
                Some(identity) => identity,
                None => {
                    if let Some(error) = recovery_error {
                        return Err(error);
                    }
                    if expected.is_none() && self.host_identity_storage_is_clean_unlocked()? {
                        return self.create_host_identity_unlocked(local_display_name);
                    }
                    return Err(
                        "host identity document is unavailable; inspect local state before registering as a new host"
                            .to_string(),
                    );
                }
            },
        };
        if let Some((expected_host_id, expected_revision)) = expected {
            if &current.host_id != expected_host_id || current.revision != expected_revision {
                return Err(
                    "host identity changed; refresh before registering this host".to_string(),
                );
            }
        }
        if recovery_error.is_some() && (!allow_damaged_recovery || expected.is_none()) {
            return Err(
                "pending host rotation is unrecoverable; explicit registration is required"
                    .to_string(),
            );
        }

        if pending_unreadable && allow_damaged_recovery {
            self.quarantine_file_unlocked(&self.host_rotation_path(), "host-rotation")?;
        }

        if !allow_damaged_recovery {
            self.load_origin_key_unlocked(&current).map_err(|error| {
                format!(
                    "host identity key is unavailable; register as a new host ({})",
                    error
                )
            })?;
        }

        let config = self.load_config_unlocked()?;
        let mut quarantine = HostRotationQuarantine::default();
        let matching_pending = pending_rotation
            .as_ref()
            .filter(|transaction| self.rotation_matches_identity(transaction, &current));
        let salvage = allow_damaged_recovery
            .then(|| {
                matching_pending.filter(|transaction| {
                    self.validate_host_rotation_payload_unlocked(transaction, &config)
                        .is_ok()
                })
            })
            .flatten();
        let setup_identity = matching_pending
            .map(|transaction| self.synthetic_previous_identity(transaction))
            .transpose()?
            .unwrap_or_else(|| current.clone());
        // Schema-1 setup journals predate host pins. Attribute them to the
        // identity that existed before rotation so they become stale drafts
        // after registration instead of silently activating old paths under
        // the new host.
        self.migrate_legacy_setup_transactions_unlocked(&setup_identity, allow_damaged_recovery)?;

        let (mut machine, current_conversations, mut materializations) = if let Some(transaction) =
            salvage
        {
            (
                transaction.machine_projects.clone(),
                transaction.conversation_bindings.clone(),
                transaction.materializations.clone(),
            )
        } else {
            let machine_result = if allow_damaged_recovery {
                self.load_bindings_document_unlocked(&config, &current)
            } else {
                self.load_bindings_unlocked(&config, &current)
            };
            let machine = match machine_result {
                Ok(machine) => machine,
                Err(_) if allow_damaged_recovery => {
                    quarantine.machine_projects = true;
                    MachineProjectState::default()
                }
                Err(error) => return Err(error),
            };
            let (conversations, conversations_degraded) = if allow_damaged_recovery {
                self.load_conversation_bindings_for_rotation_unlocked(&config, &machine, &current)?
            } else {
                (
                    self.load_conversation_bindings_document_unlocked(&config, &machine, &current)?,
                    false,
                )
            };
            quarantine.conversation_bindings =
                conversations_degraded && self.conversation_bindings_path().exists();
            let materializations = match self.load_materializations_unlocked(&config, &current) {
                Ok(materializations) => materializations,
                Err(_) if allow_damaged_recovery => {
                    quarantine.materializations = true;
                    Materializations::default()
                }
                Err(error) => return Err(error),
            };
            (machine, conversations, materializations)
        };
        // A parsed but invalid journal is not trustworthy enough to replay,
        // yet it must not permanently wedge the explicit recovery action.
        // Preserve its bytes outside the live journal path before publishing
        // a freshly validated transaction reconstructed from local state.
        if pending_rotation.is_some() && salvage.is_none() && allow_damaged_recovery {
            self.quarantine_file_unlocked(&self.host_rotation_path(), "host-rotation")?;
        }
        // Migrate legacy receipts before changing the current host so they
        // remain truthfully pinned to the installation that produced them.
        match self.load_dependency_applications_unlocked(&config, &current) {
            Ok(_) => {}
            Err(_) if allow_damaged_recovery => quarantine.dependency_applications = true,
            Err(error) => return Err(error),
        }

        let next = self.prepare_host_identity_unlocked(local_display_name)?;
        let mut next_key_guard =
            PreparedOriginKeyGuard::new(self.origin_key_path(&next.origin_key_ref));
        let timestamp = next.updated_at;
        let mut replica_map = BTreeMap::new();
        for binding in &mut machine.bindings {
            let previous_replica = binding.replica_id.clone();
            let next_replica = ReplicaId::parse(generated_named_id("replica")?)?;
            binding.host_id = next.host_id.clone();
            binding.replica_id = next_replica.clone();
            binding.state = super::domain::BindingState::Detached;
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = binding.updated_at.max(timestamp);
            replica_map.insert(previous_replica, next_replica);
        }
        machine.schema = MACHINE_PROJECT_SCHEMA_V2;
        machine.revision = machine.revision.saturating_add(1);
        machine.validate(&config)?;

        let mut conversations = current_conversations;
        conversations.host_id = next.host_id.clone();
        conversations.revision = conversations.revision.saturating_add(1);
        conversations.bindings.retain_mut(|binding| {
            let Some(next_replica) = replica_map.get(&binding.replica_id) else {
                return false;
            };
            binding.host_id = next.host_id.clone();
            binding.replica_id = next_replica.clone();
            binding.materialization_state = if binding.provider_thread_id.is_some() {
                super::domain::ConversationMaterializationState::Pending
            } else {
                super::domain::ConversationMaterializationState::StoredOnly
            };
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = binding.updated_at.max(binding.created_at).max(timestamp);
            true
        });
        conversations.validate(&next, &machine, &config)?;

        for record in &mut materializations.records {
            record.status = super::domain::MaterializationStatus::Detached;
        }
        materializations.revision = materializations.revision.saturating_add(1);
        materializations.validate(&config)?;

        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: current.host_id.clone(),
            previous_identity_revision: current.revision,
            previous_origin_key_ref: current.origin_key_ref.clone(),
            next_identity: next.clone(),
            machine_projects: machine,
            conversation_bindings: conversations,
            materializations,
            quarantine: quarantine.clone(),
            created_at: timestamp,
        };
        self.validate_host_rotation_unlocked(&transaction, &config)?;
        if let Some(previous_transaction) = salvage {
            // Settle the previous journal's deterministic quarantine paths
            // before replacing that journal with a new next-key namespace.
            // A crash leaves the old journal available to replay; once this
            // succeeds, its preserved artifacts no longer belong in the new
            // transaction's quarantine intent.
            self.apply_host_rotation_quarantine_unlocked(previous_transaction)?;
            if previous_transaction.quarantine.machine_projects {
                quarantine.machine_projects = false;
            }
            if previous_transaction.quarantine.conversation_bindings {
                quarantine.conversation_bindings = false;
            }
            if previous_transaction.quarantine.materializations {
                quarantine.materializations = false;
            }
            if previous_transaction.quarantine.dependency_applications {
                quarantine.dependency_applications = false;
            }
        }
        let mut transaction = transaction;
        transaction.quarantine = quarantine;
        let rotation_path = self.host_rotation_path();
        let publish_result = write_json_atomic_compact(
            &self.root,
            &rotation_path,
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        );
        let replacement_visible = self.host_rotation_transaction_is_visible_unlocked(&transaction);
        if publish_result.is_ok() || replacement_visible {
            next_key_guard.disarm();
        }
        publish_result?;
        self.commit_host_rotation_unlocked(&transaction)?;
        self.collect_orphaned_origin_keys_unlocked()?;
        Ok(next)
    }

    pub fn derive_origin_tokens(
        &self,
        bundle_id: &BundleId,
        replica_id: &ReplicaId,
    ) -> Result<(OriginHostToken, OriginReplicaToken), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let key = self.load_origin_key_unlocked(&identity)?;
        Ok((
            derive_origin_host_token(&key, bundle_id)?,
            derive_origin_replica_token(&key, bundle_id, replica_id)?,
        ))
    }

    /// Resolve the active replica and derive both tokens while holding the
    /// same persistence lock. Rotation therefore cannot pair a new host key
    /// with a stale replica loaded by an earlier command step.
    pub fn derive_project_origin_tokens(
        &self,
        local_project_id: &LocalProjectId,
    ) -> Result<(BundleId, OriginHostToken, OriginReplicaToken), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        let binding = machine
            .active_for(local_project_id)
            .ok_or_else(|| format!("project '{}' has no active binding", local_project_id))?;
        let key = self.load_origin_key_unlocked(&identity)?;
        Ok((
            binding.bundle_id.clone(),
            derive_origin_host_token(&key, &binding.bundle_id)?,
            derive_origin_replica_token(&key, &binding.bundle_id, &binding.replica_id)?,
        ))
    }

    pub fn derive_origin_host_token(
        &self,
        bundle_id: &BundleId,
    ) -> Result<OriginHostToken, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let key = self.load_origin_key_unlocked(&identity)?;
        derive_origin_host_token(&key, bundle_id)
    }

    pub fn resolve_origin_host_label(
        &self,
        bundle_id: &BundleId,
        origin_host_token: &OriginHostToken,
    ) -> Result<(String, bool), String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let key = self.load_origin_key_unlocked(&identity)?;
        let is_this_host = derive_origin_host_token(&key, bundle_id)? == *origin_host_token;
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
        bindings.schema = CONVERSATION_BINDINGS_SCHEMA_V2;
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

    pub fn load_origin_aliases(&self) -> Result<Vec<super::domain::OriginHostAlias>, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        Ok(self
            .load_conversation_bindings_for_aliases_unlocked(&identity)?
            .origin_aliases)
    }

    pub fn mutate_origin_aliases<T>(
        &self,
        mutate: impl FnOnce(&mut Vec<super::domain::OriginHostAlias>) -> Result<T, String>,
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

    pub fn load_config(&self) -> Result<SyncConfigV3, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_config_unlocked()
    }

    /// Full-document optimistic save used by storage/settings UI.  The
    /// caller must submit the revision it read; the returned value carries
    /// the next revision.
    pub fn save_config(&self, mut config: SyncConfigV3) -> Result<SyncConfigV3, String> {
        let _guard = persistence_guard(&self.root)?;
        let current = self.load_config_unlocked()?;
        if config.revision != current.revision {
            return Err(format!(
                "project-sync config changed (expected revision {}, current {})",
                config.revision, current.revision
            ));
        }
        config.schema = super::domain::LOCAL_SCHEMA_V3;
        config.revision = current.revision.saturating_add(1);
        config.validate()?;
        write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)?;
        Ok(config)
    }

    /// Transactional config mutation for focused project/link/recipe
    /// commands.  The closure runs while the process persistence lock is held.
    pub fn mutate_config<T>(
        &self,
        mutate: impl FnOnce(&mut SyncConfigV3) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let mut config = self.load_config_unlocked()?;
        let result = mutate(&mut config)?;
        config.schema = super::domain::LOCAL_SCHEMA_V3;
        config.revision = config.revision.saturating_add(1);
        config.validate()?;
        write_json_atomic(&self.root, &self.config_path(), &config, MAX_CONFIG_BYTES)?;
        Ok(result)
    }

    pub fn load_bindings(&self) -> Result<MachineProjectState, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        self.load_bindings_unlocked(&config, &identity)
    }

    pub fn mutate_bindings<T>(
        &self,
        mutate: impl FnOnce(&SyncConfigV3, &mut MachineProjectState) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let mut bindings = self.load_bindings_unlocked(&config, &identity)?;
        let result = mutate(&config, &mut bindings)?;
        bindings.schema = MACHINE_PROJECT_SCHEMA_V2;
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
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        self.load_materializations_unlocked(&config, &identity)
    }

    pub fn mutate_materializations<T>(
        &self,
        mutate: impl FnOnce(&SyncConfigV3, &mut Materializations) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let mut materializations = self.load_materializations_unlocked(&config, &identity)?;
        let result = mutate(&config, &mut materializations)?;
        materializations.schema = super::domain::LOCAL_SCHEMA_V3;
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

    pub fn load_dependency_applications(&self) -> Result<DependencyApplications, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        self.load_dependency_applications_unlocked(&config, &identity)
    }

    pub fn mutate_dependency_applications<T>(
        &self,
        mutate: impl FnOnce(&SyncConfigV3, &mut DependencyApplications) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let mut applications = self.load_dependency_applications_unlocked(&config, &identity)?;
        let result = mutate(&config, &mut applications)?;
        applications.schema = super::domain::LOCAL_SCHEMA_V3;
        applications.revision = applications.revision.saturating_add(1);
        applications.validate(&config)?;
        write_json_atomic(
            &self.root,
            &self.dependency_applications_path(),
            &applications,
            MAX_DEPENDENCY_APPLICATIONS_BYTES,
        )?;
        Ok(result)
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
        let identity = self.load_or_create_host_identity_unlocked()?;
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
            match self.read_setup_transaction_unlocked(&path, &identity) {
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
        let identity = self.load_or_create_host_identity_unlocked()?;
        let transaction = self
            .read_setup_transaction_unlocked(&self.setup_transaction_path(draft_id), &identity)?;
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
        identity: &HostIdentity,
    ) -> Result<Option<SetupTransaction>, String> {
        let Some(value) =
            read_json_bounded::<Value>(&self.root, path, MAX_SETUP_TRANSACTION_BYTES)?
        else {
            return Ok(None);
        };
        let (transaction, migrated) = decode_setup_transaction(value, identity)?;
        if migrated {
            write_json_atomic(&self.root, path, &transaction, MAX_SETUP_TRANSACTION_BYTES)?;
        }
        Ok(Some(transaction))
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

    /// Restore plans are immutable, generation-pinned documents.  Reusing a
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

    pub fn discard_restore_plan(&self, plan_id: &PlanId) -> Result<bool, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.restore_plan_path(plan_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("remove restore plan '{}': {}", plan_id, error)),
        }
    }

    /// Reviewed Push plans are immutable and host-pinned, just like Pull
    /// plans. Keeping them in a separate directory prevents an old schema
    /// reader from mistaking one plan shape for the other.
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

    pub fn save_dependency_plan(&self, plan: &DependencyPlan) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        plan.validate()?;
        let path = self.dependency_plan_path(&plan.plan_id);
        self.ensure_directory(path.parent().ok_or("dependency plan has no parent")?)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!("dependency plan '{}' already exists", plan.plan_id));
        }
        write_json_atomic(&self.root, &path, plan, MAX_DEPENDENCY_PLAN_BYTES)
    }

    pub fn load_dependency_plan(&self, plan_id: &PlanId) -> Result<DependencyPlan, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.dependency_plan_path(plan_id);
        let plan: DependencyPlan = read_json_bounded(&self.root, &path, MAX_DEPENDENCY_PLAN_BYTES)?
            .ok_or_else(|| format!("dependency plan '{}' does not exist", plan_id))?;
        plan.validate()?;
        Ok(plan)
    }

    fn load_config_unlocked(&self) -> Result<SyncConfigV3, String> {
        let config: SyncConfigV3 =
            read_json_bounded(&self.root, &self.config_path(), MAX_CONFIG_BYTES)?
                .unwrap_or_default();
        config.validate()?;
        Ok(config)
    }

    fn load_or_create_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
        self.recover_host_rotation_unlocked()?;
        let identity_document = self.load_host_identity_document_unlocked()?;
        if identity_document.is_some() {
            sync_directory(&self.root)?;
        }
        let mut identity = match identity_document {
            Some(identity) => identity,
            None => match self.infer_missing_host_identity_unlocked()? {
                Some(_) => {
                    return Err(
                        "host identity document is unavailable; register as a new host".to_string(),
                    )
                }
                None if self.host_identity_storage_is_clean_unlocked()? => {
                    self.create_host_identity_unlocked(None)?
                }
                None => {
                    return Err(
                        "host identity document is unavailable; register as a new host".to_string(),
                    )
                }
            },
        };
        let key = self.load_origin_key_unlocked(&identity).map_err(|error| {
            format!(
                "host identity key is unavailable; register as a new host ({})",
                error
            )
        })?;
        self.ensure_origin_key_verifier_unlocked(&mut identity, &key)?;
        self.collect_orphaned_origin_keys_unlocked()?;
        Ok(identity)
    }

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
        let mut key_guard =
            PreparedOriginKeyGuard::new(self.origin_key_path(&identity.origin_key_ref));
        let identity_path = self.host_identity_path();
        let publish_result = write_json_atomic(
            &self.root,
            &identity_path,
            &identity,
            MAX_HOST_IDENTITY_BYTES,
        );
        // `write_json_atomic` publishes by rename before syncing the parent.
        // If that final sync reports uncertainty, preserve the key whenever a
        // target is already visible; a later orphan sweep can safely decide
        // whether the published document references it.
        let replacement_visible = self
            .load_host_identity_document_unlocked()
            .ok()
            .flatten()
            .is_some_and(|visible| visible == identity)
            && sync_directory(&self.root).is_ok();
        if publish_result.is_ok() || replacement_visible {
            key_guard.disarm();
        }
        publish_result?;
        Ok(identity)
    }

    fn prepare_host_identity_unlocked(
        &self,
        local_display_name: Option<String>,
    ) -> Result<HostIdentity, String> {
        let host_id = HostId::generate()?;
        let origin_key_ref = OriginKeyRef::parse(generated_named_id("originkey")?)?;
        let key = OriginKey::generate()?;
        let timestamp = now_secs()?;
        let identity = HostIdentity {
            schema: HOST_IDENTITY_SCHEMA_V1,
            revision: 0,
            origin_key_verifier: Some(derive_origin_key_verifier(&key, &host_id)?),
            host_id,
            origin_key_ref,
            local_display_name,
            created_at: timestamp,
            updated_at: timestamp,
        };
        identity.validate()?;
        // Publish the key only after every fallible identity validation so a
        // rejected display label or clock cannot leave an orphaned secret.
        self.write_origin_key_unlocked(&identity.origin_key_ref, &key)?;
        Ok(identity)
    }

    fn ensure_origin_key_verifier_unlocked(
        &self,
        identity: &mut HostIdentity,
        key: &OriginKey,
    ) -> Result<(), String> {
        if identity.origin_key_verifier.is_some() {
            return Ok(());
        }
        identity.origin_key_verifier = Some(derive_origin_key_verifier(key, &identity.host_id)?);
        identity.revision = identity.revision.saturating_add(1);
        identity.updated_at = identity
            .updated_at
            .max(identity.created_at)
            .max(now_secs()?);
        identity.validate()?;
        write_json_atomic(
            &self.root,
            &self.host_identity_path(),
            identity,
            MAX_HOST_IDENTITY_BYTES,
        )
    }

    /// Retire only well-formed protected key files that no valid live
    /// identity or structurally valid rotation journal references. This is a
    /// crash-recovery sweep for the narrow interval between key creation and
    /// durable identity/journal publication; suspicious filesystem entries
    /// are never followed or removed by the collector.
    fn collect_orphaned_origin_keys_unlocked(&self) -> Result<(), String> {
        let directory = self.origin_keys_dir();
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("inspect protected origin-key storage: {}", error)),
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err("protected origin-key storage is not a real directory".to_string());
        }

        let mut referenced = BTreeSet::new();
        match self.load_host_identity_document_unlocked() {
            Ok(Some(identity)) => {
                sync_directory(&self.root)?;
                // An unknown marker deliberately retains every candidate key
                // until explicit registration publishes a replacement.
                if self.is_unknown_identity_marker(&identity) {
                    return Ok(());
                }
                referenced.insert(identity.origin_key_ref);
            }
            Ok(None) => {}
            // A damaged live identity may still contain the only reference to
            // a usable key. Cleanup must never turn repairable metadata damage
            // into irreversible provenance loss.
            Err(_) => return Ok(()),
        }
        match self.load_host_rotation_transaction_unlocked() {
            Ok(Some(transaction)) => {
                sync_directory(&self.root)?;
                let Ok(config) = self.load_config_unlocked() else {
                    return Ok(());
                };
                if self
                    .validate_host_rotation_payload_unlocked(&transaction, &config)
                    .is_err()
                {
                    return Ok(());
                }
                referenced.insert(transaction.previous_origin_key_ref);
                referenced.insert(transaction.next_identity.origin_key_ref);
            }
            Ok(None) => {}
            // Preserve every candidate while an unreadable recovery journal
            // is live. Explicit registration quarantines that journal first.
            Err(_) => return Ok(()),
        }

        let mut count = 0_usize;
        let mut removed = false;
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("list protected origin-key storage: {}", error))?
        {
            let entry = entry.map_err(|error| format!("list protected origin key: {}", error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "key") {
                continue;
            }
            count = count.saturating_add(1);
            if count > MAX_HOST_SCOPED_DOCUMENTS {
                return Err("too many protected origin-key files".to_string());
            }
            let Some(key_ref) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| OriginKeyRef::parse(stem).ok())
            else {
                continue;
            };
            if referenced.contains(&key_ref) {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_file() => metadata,
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("inspect protected origin key: {}", error)),
            };
            #[cfg(all(unix, not(all(target_os = "macos", not(test)))))]
            let safe_to_remove = {
                use std::os::unix::fs::MetadataExt;
                metadata.len() == ORIGIN_KEY_BYTES
                    && metadata.mode() & 0o777 == 0o600
                    && metadata.nlink() == 1
            };
            #[cfg(all(target_os = "macos", not(test)))]
            let safe_to_remove = {
                use std::os::unix::fs::MetadataExt;
                (metadata.len() == ORIGIN_KEY_BYTES
                    || (metadata.len() == MACOS_KEYCHAIN_MARKER.len() as u64
                        && macos_keychain_marker_at_path(&path)))
                    && metadata.mode() & 0o777 == 0o600
                    && metadata.nlink() == 1
            };
            #[cfg(windows)]
            let safe_to_remove =
                metadata.len() > 0 && metadata.len() <= MAX_PROTECTED_ORIGIN_KEY_BYTES;
            #[cfg(not(any(unix, windows)))]
            let safe_to_remove = false;
            if !safe_to_remove {
                continue;
            }
            // Orphan retirement must not make an otherwise healthy live
            // identity unavailable. A failed protected-store cleanup leaves
            // its marker in place and will be retried by a later sweep.
            if self.remove_origin_key_unlocked(&key_ref).is_ok() {
                removed = true;
            }
        }
        if removed {
            sync_directory(&directory)?;
        }
        Ok(())
    }

    fn write_origin_key_unlocked(
        &self,
        key_ref: &OriginKeyRef,
        key: &OriginKey,
    ) -> Result<(), String> {
        let directory = self.origin_keys_dir();
        self.ensure_directory(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    format!(
                        "secure origin key directory '{}': {}",
                        directory.display(),
                        error
                    )
                },
            )?;
        }

        #[cfg(all(target_os = "macos", not(test)))]
        {
            // A small no-secret marker makes Keychain-backed entries
            // discoverable for crash cleanup without copying the key into the
            // filesystem. If Keychain is unavailable, replace the marker with
            // the documented private-file fallback.
            self.write_origin_key_file_unlocked(key_ref, MACOS_KEYCHAIN_MARKER)?;
            return match security_framework::passwords::set_generic_password(
                MACOS_KEYCHAIN_SERVICE,
                key_ref.as_str(),
                key.as_bytes(),
            ) {
                Ok(()) => Ok(()),
                Err(keychain_error) => self.write_private_origin_key_fallback_unlocked(
                    key_ref,
                    key,
                    &keychain_error.to_string(),
                    || delete_macos_keychain_origin_key(key_ref.as_str()),
                ),
            };
        }

        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            #[cfg(windows)]
            let protected = protect_origin_key_for_current_user(key.as_bytes())?;
            #[cfg(windows)]
            let stored_bytes = protected.as_slice();
            #[cfg(not(windows))]
            let stored_bytes = key.as_bytes().as_slice();
            self.write_origin_key_file_unlocked(key_ref, stored_bytes)
        }
    }

    fn write_private_origin_key_fallback_unlocked(
        &self,
        key_ref: &OriginKeyRef,
        key: &OriginKey,
        protected_store_error: &str,
        confirm_protected_item_cleanup: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        confirm_protected_item_cleanup().map_err(|cleanup_error| {
            format!(
                "macOS Keychain is unavailable ({}), and its origin-key item could not be safely retired ({}); private-file fallback was not created",
                protected_store_error, cleanup_error
            )
        })?;
        self.remove_origin_key_file_unlocked(key_ref)?;
        self.write_origin_key_file_unlocked(key_ref, key.as_bytes())
            .map_err(|fallback_error| {
                format!(
                    "macOS Keychain is unavailable ({}); private origin-key fallback also failed: {}",
                    protected_store_error, fallback_error
                )
            })
    }

    fn write_origin_key_file_unlocked(
        &self,
        key_ref: &OriginKeyRef,
        stored_bytes: &[u8],
    ) -> Result<(), String> {
        let directory = self.origin_keys_dir();
        let path = self.origin_key_path(key_ref);
        ensure_no_symlinks(&self.root, &path)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&path)
            .map_err(|error| format!("create protected origin key '{}': {}", key_ref, error))?;
        // Arm cleanup as soon as create_new succeeds. Permission, write, or
        // durability failures must not leave a partial secret at its final
        // path where the conservative orphan collector would preserve it.
        let mut partial_key_guard = PreparedOriginKeyGuard::new(path);
        let write_result = (|| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|error| {
                        format!("secure protected origin key '{}': {}", key_ref, error)
                    })?;
            }
            file.write_all(stored_bytes)
                .map_err(|error| format!("write protected origin key '{}': {}", key_ref, error))?;
            file.sync_all()
                .map_err(|error| format!("sync protected origin key '{}': {}", key_ref, error))
        })();
        drop(file);
        write_result?;
        sync_directory(&directory)?;
        partial_key_guard.disarm();
        Ok(())
    }

    fn remove_origin_key_file_unlocked(&self, key_ref: &OriginKeyRef) -> Result<(), String> {
        let path = self.origin_key_path(key_ref);
        ensure_no_symlinks(&self.root, &path)?;
        let path_metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A previous remove may have succeeded before its directory
                // sync reported uncertainty. Re-sync the visible absence
                // before a rotation journal can retire its cleanup intent.
                return sync_directory(&self.origin_keys_dir());
            }
            Err(error) => {
                return Err(format!(
                    "inspect protected origin key '{}': {}",
                    key_ref, error
                ))
            }
        };
        if !path_metadata.file_type().is_file() || path_metadata.file_type().is_symlink() {
            return Err(format!(
                "protected origin key '{}' is not a regular file",
                key_ref
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&path).map_err(|error| {
            format!(
                "open protected origin key '{}' for retirement: {}",
                key_ref, error
            )
        })?;
        let metadata = file.metadata().map_err(|error| {
            format!(
                "inspect protected origin key '{}' for retirement: {}",
                key_ref, error
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "protected origin key '{}' is not a regular file",
                key_ref
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(format!(
                    "protected origin key '{}' has unsafe hard links",
                    key_ref
                ));
            }
            if metadata.dev() != path_metadata.dev() || metadata.ino() != path_metadata.ino() {
                return Err(format!(
                    "protected origin key '{}' changed during retirement",
                    key_ref
                ));
            }
        }
        #[cfg(windows)]
        {
            let identity_description = format!("protected origin key '{}'", key_ref);
            let opened_identity = windows_file_identity(&file, &identity_description)?;
            if opened_identity.2 != 1 {
                return Err(format!(
                    "protected origin key '{}' has unsafe hard links",
                    key_ref
                ));
            }
            let current_file = options.open(&path).map_err(|error| {
                format!(
                    "reopen protected origin key '{}' for retirement: {}",
                    key_ref, error
                )
            })?;
            let current_identity = windows_file_identity(&current_file, &identity_description)?;
            if current_identity.2 != 1 {
                return Err(format!(
                    "protected origin key '{}' has unsafe hard links",
                    key_ref
                ));
            }
            if opened_identity.0 != current_identity.0 || opened_identity.1 != current_identity.1 {
                return Err(format!(
                    "protected origin key '{}' changed during retirement",
                    key_ref
                ));
            }
        }
        #[cfg(all(unix, not(all(target_os = "macos", not(test)))))]
        let expected_representation = metadata.len() == ORIGIN_KEY_BYTES;
        #[cfg(all(target_os = "macos", not(test)))]
        let expected_representation = metadata.len() == ORIGIN_KEY_BYTES
            || metadata.len() == MACOS_KEYCHAIN_MARKER.len() as u64;
        #[cfg(windows)]
        let expected_representation =
            metadata.len() > 0 && metadata.len() <= MAX_PROTECTED_ORIGIN_KEY_BYTES;
        #[cfg(not(any(unix, windows)))]
        let expected_representation = false;
        if !expected_representation {
            return Err(format!(
                "protected origin key '{}' has an invalid stored representation",
                key_ref
            ));
        }
        drop(file);
        match fs::remove_file(&path) {
            Ok(()) => sync_directory(&self.origin_keys_dir()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                sync_directory(&self.origin_keys_dir())
            }
            Err(error) => Err(format!(
                "retire protected origin key '{}': {}",
                key_ref, error
            )),
        }
    }

    fn remove_origin_key_unlocked(&self, key_ref: &OriginKeyRef) -> Result<(), String> {
        #[cfg(all(target_os = "macos", not(test)))]
        delete_macos_keychain_origin_key(key_ref.as_str())?;
        self.remove_origin_key_file_unlocked(key_ref)
    }

    fn load_origin_key_unlocked(&self, identity: &HostIdentity) -> Result<OriginKey, String> {
        let path = self.origin_key_path(&identity.origin_key_ref);
        ensure_no_symlinks(&self.root, &path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&path).map_err(|error| {
            format!(
                "open protected origin key '{}': {}",
                identity.origin_key_ref, error
            )
        })?;
        // Validate the opened descriptor rather than path metadata so a
        // same-account replacement between inspection and open cannot evade
        // type, size, link-count, or permission checks.
        let metadata = file.metadata().map_err(|error| {
            format!(
                "inspect protected origin key '{}': {}",
                identity.origin_key_ref, error
            )
        })?;
        #[cfg(all(unix, not(all(target_os = "macos", not(test)))))]
        let invalid_size = metadata.len() != ORIGIN_KEY_BYTES;
        #[cfg(all(target_os = "macos", not(test)))]
        let invalid_size = metadata.len() != ORIGIN_KEY_BYTES
            && metadata.len() != MACOS_KEYCHAIN_MARKER.len() as u64;
        #[cfg(windows)]
        let invalid_size = metadata.len() == 0 || metadata.len() > MAX_PROTECTED_ORIGIN_KEY_BYTES;
        #[cfg(not(any(unix, windows)))]
        let invalid_size = true;
        if !metadata.file_type().is_file() || invalid_size {
            return Err(format!(
                "protected origin key '{}' is not a valid protected regular file",
                identity.origin_key_ref
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o777 != 0o600 || metadata.nlink() != 1 {
                return Err(format!(
                    "protected origin key '{}' has unsafe permissions or links",
                    identity.origin_key_ref
                ));
            }
        }
        let mut stored_bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(metadata.len().saturating_add(1))
            .read_to_end(&mut stored_bytes)
            .map_err(|error| {
                format!(
                    "read protected origin key '{}': {}",
                    identity.origin_key_ref, error
                )
            })?;
        if stored_bytes.len() as u64 != metadata.len() {
            return Err(format!(
                "protected origin key '{}' changed while reading",
                identity.origin_key_ref
            ));
        }
        #[cfg(windows)]
        let bytes = unprotect_origin_key_for_current_user(&stored_bytes)?;
        #[cfg(all(target_os = "macos", not(test)))]
        let bytes = if stored_bytes == MACOS_KEYCHAIN_MARKER {
            security_framework::passwords::get_generic_password(
                MACOS_KEYCHAIN_SERVICE,
                identity.origin_key_ref.as_str(),
            )
            .map_err(|error| {
                format!(
                    "open protected origin key '{}' from macOS Keychain: {}",
                    identity.origin_key_ref, error
                )
            })?
        } else {
            stored_bytes
        };
        #[cfg(all(not(windows), not(all(target_os = "macos", not(test)))))]
        let bytes = stored_bytes;
        let key = OriginKey::from_bytes(&bytes)?;
        if let Some(expected) = &identity.origin_key_verifier {
            let actual = derive_origin_key_verifier(&key, &identity.host_id)?;
            if &actual != expected {
                return Err(format!(
                    "protected origin key '{}' does not match this host identity",
                    identity.origin_key_ref
                ));
            }
        }
        Ok(key)
    }

    fn load_host_rotation_transaction_unlocked(
        &self,
    ) -> Result<Option<HostRotationTransaction>, String> {
        self.load_host_rotation_transaction_classified_unlocked()
            .map_err(BoundedJsonReadError::into_message)
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

    fn host_rotation_transaction_is_visible_unlocked(
        &self,
        expected: &HostRotationTransaction,
    ) -> bool {
        let visible = self
            .load_host_rotation_transaction_unlocked()
            .ok()
            .flatten()
            .as_ref()
            .is_some_and(|visible| visible == expected);
        visible && sync_directory(&self.root).is_ok()
    }

    fn synthetic_previous_identity(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<HostIdentity, String> {
        let identity = HostIdentity {
            schema: HOST_IDENTITY_SCHEMA_V1,
            revision: transaction.previous_identity_revision,
            host_id: transaction.previous_host_id.clone(),
            origin_key_ref: transaction.previous_origin_key_ref.clone(),
            origin_key_verifier: None,
            local_display_name: None,
            created_at: 0,
            updated_at: 0,
        };
        identity.validate()?;
        Ok(identity)
    }

    fn synthetic_missing_identity(&self, host_id: HostId) -> Result<HostIdentity, String> {
        let identity = HostIdentity {
            schema: HOST_IDENTITY_SCHEMA_V1,
            revision: 0,
            host_id,
            origin_key_ref: OriginKeyRef::parse(MISSING_ORIGIN_KEY_REF)?,
            origin_key_verifier: None,
            local_display_name: None,
            created_at: 0,
            updated_at: 0,
        };
        identity.validate()?;
        Ok(identity)
    }

    fn is_unknown_identity_marker(&self, identity: &HostIdentity) -> bool {
        identity.origin_key_ref.as_str() == UNKNOWN_ORIGIN_KEY_REF
            && identity.origin_key_verifier.is_none()
    }

    fn create_unknown_identity_marker_unlocked(&self) -> Result<HostIdentity, String> {
        let timestamp = now_secs()?;
        let identity = HostIdentity {
            schema: HOST_IDENTITY_SCHEMA_V1,
            revision: 0,
            host_id: HostId::generate()?,
            origin_key_ref: OriginKeyRef::parse(UNKNOWN_ORIGIN_KEY_REF)?,
            origin_key_verifier: None,
            local_display_name: None,
            created_at: timestamp,
            updated_at: timestamp,
        };
        identity.validate()?;
        write_json_atomic(
            &self.root,
            &self.host_identity_path(),
            &identity,
            MAX_HOST_IDENTITY_BYTES,
        )?;
        Ok(identity)
    }

    fn infer_missing_host_identity_unlocked(&self) -> Result<Option<HostIdentity>, String> {
        let mut primary = BTreeSet::new();
        self.collect_document_host_ids_unlocked(
            &self.machine_projects_path(),
            MAX_BINDINGS_BYTES,
            &mut primary,
        )?;
        self.collect_document_host_ids_unlocked(
            &self.conversation_bindings_path(),
            MAX_CONVERSATION_BINDINGS_BYTES,
            &mut primary,
        )?;
        if primary.len() > 1 {
            return Err(
                "host identity is missing and active local state names multiple hosts".to_string(),
            );
        }
        if let Some(host_id) = primary.into_iter().next() {
            return self.synthetic_missing_identity(host_id).map(Some);
        }

        // Receipts, plans, and setup journals are secondary evidence because
        // they may legitimately retain earlier host pins after rotation. If
        // they disagree and no active binding document survives, guessing a
        // current identity would make stale approvals look current.
        let mut secondary = BTreeSet::new();
        self.collect_document_host_ids_unlocked(
            &self.materializations_path(),
            MAX_MATERIALIZATIONS_BYTES,
            &mut secondary,
        )?;
        self.collect_document_host_ids_unlocked(
            &self.dependency_applications_path(),
            MAX_DEPENDENCY_APPLICATIONS_BYTES,
            &mut secondary,
        )?;
        self.collect_directory_host_ids_unlocked(
            &self.root.join("setup_transactions"),
            MAX_SETUP_TRANSACTION_BYTES,
            &mut secondary,
        )?;
        self.collect_directory_host_ids_unlocked(
            &self.root.join("restore_plans"),
            MAX_RESTORE_PLAN_BYTES,
            &mut secondary,
        )?;
        self.collect_directory_host_ids_unlocked(
            &self.root.join("dependency_plans"),
            MAX_DEPENDENCY_PLAN_BYTES,
            &mut secondary,
        )?;
        match secondary.len() {
            0 => Ok(None),
            1 => self
                .synthetic_missing_identity(secondary.into_iter().next().unwrap())
                .map(Some),
            _ => Err(
                "host identity is missing and retained local records name multiple hosts"
                    .to_string(),
            ),
        }
    }

    fn host_identity_storage_is_clean_unlocked(&self) -> Result<bool, String> {
        for (directory, label) in [
            (self.origin_keys_dir(), "protected origin-key storage"),
            (
                self.root.join("host_recovery_quarantine"),
                "host recovery quarantine",
            ),
        ] {
            let metadata = match fs::symlink_metadata(&directory) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("inspect {}: {}", label, error)),
            };
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(format!("{} is not a real directory", label));
            }
            if fs::read_dir(&directory)
                .map_err(|error| format!("list {}: {}", label, error))?
                .next()
                .is_some()
            {
                return Ok(false);
            }
        }

        // A current-format primary document cannot be assigned to a freshly
        // generated host merely because corruption removed every parseable
        // host_id. Only the explicitly supported hostless migration formats
        // are compatible with a true first run.
        if self.path_exists_unlocked(&self.host_rotation_path())?
            || self.path_exists_unlocked(&self.conversation_bindings_path())?
        {
            return Ok(false);
        }
        let config = match self.load_config_unlocked() {
            Ok(config) => config,
            Err(_) => return Ok(false),
        };
        let legacy_identity =
            self.synthetic_missing_identity(HostId::parse("00000000000000000000000000000000")?)?;
        if !self.machine_projects_absent_or_valid_legacy_unlocked(&config, &legacy_identity)?
            || !self.materializations_absent_or_valid_legacy_unlocked(&config, &legacy_identity)?
            || !self.dependency_applications_absent_or_valid_legacy_unlocked(
                &config,
                &legacy_identity,
            )?
            || !self.setup_transactions_absent_or_valid_legacy_unlocked(&legacy_identity)?
        {
            return Ok(false);
        }
        for directory in [
            self.root.join("restore_plans"),
            self.root.join("dependency_plans"),
        ] {
            if self.directory_has_json_documents_unlocked(&directory)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn path_exists_unlocked(&self, path: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("inspect host-scoped local state: {}", error)),
        }
    }

    fn machine_projects_absent_or_valid_legacy_unlocked(
        &self,
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<bool, String> {
        let path = self.machine_projects_path();
        if !self.path_exists_unlocked(&path)? {
            return Ok(true);
        }
        let value = match read_json_bounded::<Value>(&self.root, &path, MAX_BINDINGS_BYTES) {
            Ok(Some(value)) => value,
            Ok(None) | Err(_) => return Ok(false),
        };
        if value.get("schema").and_then(Value::as_u64) != Some(MACHINE_PROJECT_SCHEMA_V1 as u64) {
            return Ok(false);
        }
        Ok(decode_machine_project_state(value, identity, config)
            .is_ok_and(|(_, migrated)| migrated))
    }

    fn materializations_absent_or_valid_legacy_unlocked(
        &self,
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<bool, String> {
        let path = self.materializations_path();
        if !self.path_exists_unlocked(&path)? {
            return Ok(true);
        }
        let mut value =
            match read_json_bounded::<Value>(&self.root, &path, MAX_MATERIALIZATIONS_BYTES) {
                Ok(Some(value)) => value,
                Ok(None) | Err(_) => return Ok(false),
            };
        if inject_host_id_into_records(&mut value, &identity.host_id).is_err() {
            return Ok(false);
        }
        let Ok(document) = serde_json::from_value::<Materializations>(value) else {
            return Ok(false);
        };
        Ok(document.validate(config).is_ok())
    }

    fn dependency_applications_absent_or_valid_legacy_unlocked(
        &self,
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<bool, String> {
        let path = self.dependency_applications_path();
        if !self.path_exists_unlocked(&path)? {
            return Ok(true);
        }
        let mut value = match read_json_bounded::<Value>(
            &self.root,
            &path,
            MAX_DEPENDENCY_APPLICATIONS_BYTES,
        ) {
            Ok(Some(value)) => value,
            Ok(None) | Err(_) => return Ok(false),
        };
        if inject_host_id_into_records(&mut value, &identity.host_id).is_err() {
            return Ok(false);
        }
        let Ok(document) = serde_json::from_value::<DependencyApplications>(value) else {
            return Ok(false);
        };
        Ok(document.validate(config).is_ok())
    }

    fn setup_transactions_absent_or_valid_legacy_unlocked(
        &self,
        identity: &HostIdentity,
    ) -> Result<bool, String> {
        let directory = self.root.join("setup_transactions");
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(format!("inspect setup transaction storage: {}", error)),
        }
        let mut count = 0_usize;
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
                return Ok(false);
            }
            let value =
                match read_json_bounded::<Value>(&self.root, &path, MAX_SETUP_TRANSACTION_BYTES) {
                    Ok(Some(value)) => value,
                    Ok(None) | Err(_) => return Ok(false),
                };
            if value.get("schema").and_then(Value::as_u64)
                != Some(SETUP_TRANSACTION_SCHEMA_V1 as u64)
                || decode_setup_transaction(value, identity).is_err()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn directory_has_json_documents_unlocked(&self, directory: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("inspect host-scoped document storage: {}", error)),
        }
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("list host-scoped document storage: {}", error))?
        {
            let path = entry
                .map_err(|error| format!("list host-scoped document: {}", error))?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn collect_document_host_ids_unlocked(
        &self,
        path: &Path,
        max_bytes: u64,
        host_ids: &mut BTreeSet<HostId>,
    ) -> Result<(), String> {
        if let Some(value) = read_json_bounded::<Value>(&self.root, path, max_bytes)? {
            collect_host_ids(&value, host_ids)?;
        }
        Ok(())
    }

    fn collect_directory_host_ids_unlocked(
        &self,
        directory: &Path,
        max_bytes: u64,
        host_ids: &mut BTreeSet<HostId>,
    ) -> Result<(), String> {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err("host-scoped document storage is not a real directory".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("inspect host-scoped document storage: {}", error)),
        }
        let mut count = 0_usize;
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("list host-scoped document storage: {}", error))?
        {
            let path = entry
                .map_err(|error| format!("list host-scoped document: {}", error))?
                .path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            count = count.saturating_add(1);
            if count > MAX_HOST_SCOPED_DOCUMENTS {
                return Err("too many host-scoped local documents".to_string());
            }
            self.collect_document_host_ids_unlocked(&path, max_bytes, host_ids)?;
        }
        Ok(())
    }

    fn migrate_legacy_setup_transactions_unlocked(
        &self,
        identity: &HostIdentity,
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
        let mut migrations = Vec::new();
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
                    .and_then(|value| decode_setup_transaction(value, identity));
            match decoded {
                Ok((transaction, true)) => migrations.push((path, transaction)),
                Ok((_, false)) => {}
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
        // any setup journal. Imported recovery may then salvage valid legacy
        // entries and quarantine only the entries it could not validate.
        for (path, transaction) in migrations {
            write_json_atomic(&self.root, &path, &transaction, MAX_SETUP_TRANSACTION_BYTES)?;
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
            category, transaction.next_identity.origin_key_ref
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

    fn rotation_matches_identity(
        &self,
        transaction: &HostRotationTransaction,
        identity: &HostIdentity,
    ) -> bool {
        (identity.host_id == transaction.previous_host_id
            && identity.revision == transaction.previous_identity_revision
            && identity.origin_key_ref == transaction.previous_origin_key_ref)
            || identity == &transaction.next_identity
    }

    fn identity_descends_from_rotation_next(
        &self,
        transaction: &HostRotationTransaction,
        identity: &HostIdentity,
    ) -> bool {
        identity.schema == transaction.next_identity.schema
            && identity.host_id == transaction.next_identity.host_id
            && identity.origin_key_ref == transaction.next_identity.origin_key_ref
            && identity.origin_key_verifier == transaction.next_identity.origin_key_verifier
            && identity.created_at == transaction.next_identity.created_at
            && identity.revision >= transaction.next_identity.revision
            && identity.updated_at >= transaction.next_identity.updated_at
    }

    fn rotation_is_anchored_without_identity_unlocked(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<bool, String> {
        let mut primary = BTreeSet::new();
        self.collect_document_host_ids_unlocked(
            &self.machine_projects_path(),
            MAX_BINDINGS_BYTES,
            &mut primary,
        )?;
        self.collect_document_host_ids_unlocked(
            &self.conversation_bindings_path(),
            MAX_CONVERSATION_BINDINGS_BYTES,
            &mut primary,
        )?;
        if !primary.is_empty() {
            return Ok(primary.iter().all(|host_id| {
                host_id == &transaction.previous_host_id
                    || host_id == &transaction.next_identity.host_id
            }));
        }

        let mut secondary = BTreeSet::new();
        self.collect_document_host_ids_unlocked(
            &self.materializations_path(),
            MAX_MATERIALIZATIONS_BYTES,
            &mut secondary,
        )?;
        self.collect_document_host_ids_unlocked(
            &self.dependency_applications_path(),
            MAX_DEPENDENCY_APPLICATIONS_BYTES,
            &mut secondary,
        )?;
        self.collect_directory_host_ids_unlocked(
            &self.root.join("setup_transactions"),
            MAX_SETUP_TRANSACTION_BYTES,
            &mut secondary,
        )?;
        self.collect_directory_host_ids_unlocked(
            &self.root.join("restore_plans"),
            MAX_RESTORE_PLAN_BYTES,
            &mut secondary,
        )?;
        self.collect_directory_host_ids_unlocked(
            &self.root.join("dependency_plans"),
            MAX_DEPENDENCY_PLAN_BYTES,
            &mut secondary,
        )?;
        Ok(secondary.iter().all(|host_id| {
            host_id == &transaction.previous_host_id
                || host_id == &transaction.next_identity.host_id
        }))
    }

    fn recover_host_rotation_unlocked(&self) -> Result<HostRotationRecoveryState, String> {
        let transaction = self.load_host_rotation_transaction_unlocked()?;
        let Some(transaction) = transaction else {
            return Ok(HostRotationRecoveryState::Ready);
        };
        // Keep durability failures distinct from parse/validation corruption:
        // callers may quarantine the latter, never the former.
        sync_directory(&self.root)?;
        let cleanup_only = if let Some(current) = self.load_host_identity_document_unlocked()? {
            if current.host_id == transaction.previous_host_id {
                if current.revision != transaction.previous_identity_revision
                    || current.origin_key_ref != transaction.previous_origin_key_ref
                {
                    return Err(
                        "pending host rotation no longer matches the previous identity".to_string(),
                    );
                }
                false
            } else if self.identity_descends_from_rotation_next(&transaction, &current) {
                true
            } else {
                return Err(
                    "pending host rotation belongs to an unrelated host identity".to_string(),
                );
            }
        } else if !self.rotation_is_anchored_without_identity_unlocked(&transaction)? {
            return Err("pending host rotation does not match local host-scoped state".to_string());
        } else {
            false
        };
        if cleanup_only {
            self.validate_host_rotation_identity_transition_unlocked(&transaction)?;
            self.load_origin_key_unlocked(&transaction.next_identity)?;
            // The visible next identity may be the result of an atomic rename
            // whose first directory sync failed. Re-establish its durability
            // before deleting the only previous identity key.
            sync_directory(&self.root)?;
            self.finish_host_rotation_cleanup_unlocked(&transaction)
        } else {
            let config = self.load_config_unlocked()?;
            self.validate_host_rotation_unlocked(&transaction, &config)?;
            self.commit_host_rotation_unlocked(&transaction)
        }
    }

    fn validate_host_rotation_unlocked(
        &self,
        transaction: &HostRotationTransaction,
        config: &SyncConfigV3,
    ) -> Result<(), String> {
        self.validate_host_rotation_payload_unlocked(transaction, config)?;
        self.load_origin_key_unlocked(&transaction.next_identity)?;
        Ok(())
    }

    fn validate_host_rotation_payload_unlocked(
        &self,
        transaction: &HostRotationTransaction,
        config: &SyncConfigV3,
    ) -> Result<(), String> {
        self.validate_host_rotation_identity_transition_unlocked(transaction)?;
        transaction.machine_projects.validate(config)?;
        if transaction.machine_projects.bindings.iter().any(|binding| {
            binding.host_id != transaction.next_identity.host_id
                || binding.state != super::domain::BindingState::Detached
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
                    == super::domain::ConversationMaterializationState::Resumable
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
            .any(|record| record.status != super::domain::MaterializationStatus::Detached)
        {
            return Err("host rotation contains a live materialization record".to_string());
        }
        Ok(())
    }

    fn validate_host_rotation_identity_transition_unlocked(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<(), String> {
        if transaction.schema != 1
            || transaction.previous_host_id == transaction.next_identity.host_id
            || transaction.previous_origin_key_ref == transaction.next_identity.origin_key_ref
            || transaction.next_identity.origin_key_verifier.is_none()
        {
            return Err("invalid host rotation transaction".to_string());
        }
        transaction.next_identity.validate()
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
        if transaction.quarantine.dependency_applications {
            self.quarantine_host_rotation_file_unlocked(
                transaction,
                &self.dependency_applications_path(),
                "dependency-applications",
            )?;
        }
        Ok(())
    }

    fn commit_host_rotation_unlocked(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<HostRotationRecoveryState, String> {
        // Quarantine intent is part of the durable transaction. Replaying it
        // before any replacement write preserves damaged originals across a
        // crash and ensures dependency receipts cannot remain malformed after
        // the journal is retired. Already-written replacement documents are
        // recognized and left in place, making recovery idempotent.
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
        self.finish_host_rotation_cleanup_unlocked(transaction)
    }

    fn finish_host_rotation_cleanup_unlocked(
        &self,
        transaction: &HostRotationTransaction,
    ) -> Result<HostRotationRecoveryState, String> {
        // Preserve ordering even when this helper follows a partially failed
        // identity publication: replacements become durable before cleanup of
        // either the previous key or the recovery journal.
        sync_directory(&self.root)?;
        if transaction.previous_origin_key_ref != transaction.next_identity.origin_key_ref {
            if self
                .remove_origin_key_unlocked(&transaction.previous_origin_key_ref)
                .is_err()
            {
                // All host-scoped documents and the next identity are already
                // durable. Keep the journal as a retryable reference to the
                // previous privacy-sensitive key instead of misclassifying the
                // valid next identity as imported state.
                return Ok(HostRotationRecoveryState::CleanupPending);
            }
        }
        let path = self.host_rotation_path();
        ensure_no_symlinks(&self.root, &path)?;
        match fs::remove_file(&path) {
            Ok(()) => {
                sync_directory(&self.root)?;
                Ok(HostRotationRecoveryState::Ready)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                sync_directory(&self.root)?;
                Ok(HostRotationRecoveryState::Ready)
            }
            Err(_) if self.host_rotation_transaction_is_visible_unlocked(transaction) => {
                Ok(HostRotationRecoveryState::CleanupPending)
            }
            Err(error) => Err(format!("retire host rotation transaction: {}", error)),
        }
    }

    fn load_bindings_unlocked(
        &self,
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<MachineProjectState, String> {
        let bindings = self.load_bindings_document_unlocked(config, identity)?;
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
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<MachineProjectState, String> {
        let path = self.machine_projects_path();
        let Some(value) = read_json_bounded::<Value>(&self.root, &path, MAX_BINDINGS_BYTES)? else {
            return Ok(MachineProjectState::default());
        };
        let (bindings, migrated) = decode_machine_project_state(value, identity, config)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        if migrated {
            write_json_atomic(&self.root, &path, &bindings, MAX_BINDINGS_BYTES)?;
        }
        Ok(bindings)
    }

    fn load_materializations_unlocked(
        &self,
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<Materializations, String> {
        let path = self.materializations_path();
        let Some(mut value) =
            read_json_bounded::<Value>(&self.root, &path, MAX_MATERIALIZATIONS_BYTES)?
        else {
            return Ok(Materializations::default());
        };
        let migrated = inject_host_id_into_records(&mut value, &identity.host_id)?;
        let materializations: Materializations = serde_json::from_value(value)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        materializations.validate(config)?;
        if migrated {
            write_json_atomic(
                &self.root,
                &path,
                &materializations,
                MAX_MATERIALIZATIONS_BYTES,
            )?;
        }
        Ok(materializations)
    }

    fn load_dependency_applications_unlocked(
        &self,
        config: &SyncConfigV3,
        identity: &HostIdentity,
    ) -> Result<DependencyApplications, String> {
        let path = self.dependency_applications_path();
        let Some(mut value) =
            read_json_bounded::<Value>(&self.root, &path, MAX_DEPENDENCY_APPLICATIONS_BYTES)?
        else {
            return Ok(DependencyApplications::default());
        };
        let migrated = inject_host_id_into_records(&mut value, &identity.host_id)?;
        let applications: DependencyApplications = serde_json::from_value(value)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        applications.validate(config)?;
        if migrated {
            write_json_atomic(
                &self.root,
                &path,
                &applications,
                MAX_DEPENDENCY_APPLICATIONS_BYTES,
            )?;
        }
        Ok(applications)
    }

    fn read_conversation_bindings_unlocked(
        &self,
    ) -> Result<Option<ConversationBindings>, String> {
        read_json_bounded(
            &self.root,
            &self.conversation_bindings_path(),
            MAX_CONVERSATION_BINDINGS_BYTES,
        )
    }

    fn load_conversation_bindings_document_unlocked(
        &self,
        config: &SyncConfigV3,
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
        if document.schema != CONVERSATION_BINDINGS_SCHEMA_V2
            || document.bindings.len() > super::domain::MAX_CONVERSATION_BINDINGS
            || document.origin_aliases.len() > super::domain::MAX_ORIGIN_ALIASES
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
    fn load_conversation_bindings_for_rotation_unlocked(
        &self,
        config: &SyncConfigV3,
        machine: &MachineProjectState,
        identity: &HostIdentity,
    ) -> Result<(ConversationBindings, bool), String> {
        let loaded = read_json_bounded::<ConversationBindings>(
            &self.root,
            &self.conversation_bindings_path(),
            MAX_CONVERSATION_BINDINGS_BYTES,
        );
        let bindings = match loaded {
            Ok(Some(bindings)) => bindings,
            Ok(None) => return Ok((ConversationBindings::empty(identity.host_id.clone()), false)),
            Err(_) => return Ok((ConversationBindings::empty(identity.host_id.clone()), true)),
        };
        if bindings.validate(identity, machine, config).is_ok() {
            return Ok((bindings, false));
        }

        // A partially committed rotation can leave the machine document on
        // the journal's next host while the identity or conversation document
        // is still on the previous host. Rebase independently valid rows onto
        // the uniquely matching project/profile scope, but keep them Pending
        // during the final rotation so this recovery never grants Continue.
        let salvage_host_id = machine
            .bindings
            .first()
            .map(|binding| binding.host_id.clone())
            .unwrap_or_else(|| identity.host_id.clone());
        let salvage_identity = if salvage_host_id == identity.host_id {
            identity.clone()
        } else {
            self.synthetic_missing_identity(salvage_host_id.clone())?
        };
        let mut salvaged = ConversationBindings::empty(salvage_host_id.clone());
        salvaged.revision = bindings.revision;

        // A stale association must not erase unrelated conversation identity.
        // Rebuild the document one independently valid, uniquely scoped row
        // at a time.
        let may_salvage_bindings = bindings.schema == CONVERSATION_BINDINGS_SCHEMA_V2
            && bindings.bindings.len() <= super::domain::MAX_CONVERSATION_BINDINGS;
        if may_salvage_bindings {
            let profiles = machine
                .profiles
                .iter()
                .map(|profile| (profile.profile_id.clone(), profile))
                .collect::<BTreeMap<_, _>>();
            let mut binding_keys = HashSet::new();
            let mut provider_owners = HashSet::new();
            for mut binding in bindings.bindings {
                if binding.validate().is_err()
                    || !profiles.contains_key(&binding.provider_profile_id)
                {
                    continue;
                }
                let candidates = machine
                    .bindings
                    .iter()
                    .filter(|candidate| {
                        candidate.local_project_id == binding.local_project_id
                            && candidate
                                .profile_ids
                                .values()
                                .any(|profile_id| profile_id == &binding.provider_profile_id)
                            && (candidate.state != super::domain::BindingState::Active
                                || config.project(&binding.local_project_id).is_some())
                    })
                    .collect::<Vec<_>>();
                let project_binding = if let Some(exact) = candidates
                    .iter()
                    .find(|candidate| candidate.replica_id == binding.replica_id)
                {
                    *exact
                } else if candidates.len() == 1 {
                    candidates[0]
                } else {
                    continue;
                };
                binding.host_id = salvage_host_id.clone();
                binding.replica_id = project_binding.replica_id.clone();
                let key = (
                    binding.host_id.clone(),
                    binding.replica_id.clone(),
                    binding.local_project_id.clone(),
                    binding.provider_profile_id.clone(),
                    binding.thread_id.clone(),
                );
                let provider_owner = binding.provider_thread_id.as_ref().map(|thread_id| {
                    (
                        binding.host_id.clone(),
                        binding.provider_profile_id.clone(),
                        thread_id.clone(),
                    )
                });
                let scope_valid = project_binding.host_id == salvage_host_id
                    && !binding_keys.contains(&key)
                    && provider_owner
                        .as_ref()
                        .is_none_or(|owner| !provider_owners.contains(owner));
                if !scope_valid {
                    continue;
                }
                binding_keys.insert(key);
                if let Some(owner) = provider_owner {
                    provider_owners.insert(owner);
                }
                salvaged.bindings.push(binding);
            }
        }

        if bindings.origin_aliases.len() <= super::domain::MAX_ORIGIN_ALIASES {
            let mut alias_keys = HashSet::new();
            for alias in bindings.origin_aliases {
                let key = (alias.bundle_id.clone(), alias.origin_host_token.clone());
                if alias.validate().is_ok() && alias_keys.insert(key) {
                    salvaged.origin_aliases.push(alias);
                }
            }
        }
        salvaged.validate(&salvage_identity, machine, config)?;
        Ok((salvaged, true))
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
        ensure_no_symlinks(&self.root, &self.root)
    }

    fn ensure_directory(&self, directory: &Path) -> Result<(), String> {
        let relative = directory.strip_prefix(&self.root).map_err(|_| {
            format!(
                "schema-3 path '{}' escapes '{}'",
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
                        "schema-3 directory '{}' is not a real directory",
                        current.display()
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {
                            let parent = current.parent().ok_or_else(|| {
                                format!("schema-3 directory '{}' has no parent", current.display())
                            })?;
                            sync_directory(parent)?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            ensure_real_directory(&current)?;
                            let parent = current.parent().ok_or_else(|| {
                                format!("schema-3 directory '{}' has no parent", current.display())
                            })?;
                            sync_directory(parent)?;
                        }
                        Err(error) => {
                            return Err(format!(
                                "create schema-3 directory '{}': {}",
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

#[cfg(all(target_os = "macos", not(test)))]
fn macos_keychain_marker_at_path(path: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut options = OpenOptions::new();
    options.read(true).custom_flags(libc::O_NOFOLLOW);
    let Ok(file) = options.open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if !metadata.file_type().is_file()
        || metadata.len() != MACOS_KEYCHAIN_MARKER.len() as u64
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return false;
    }
    let mut bytes = Vec::with_capacity(MACOS_KEYCHAIN_MARKER.len());
    file.take(MACOS_KEYCHAIN_MARKER.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .is_ok()
        && bytes == MACOS_KEYCHAIN_MARKER
}

#[cfg(all(target_os = "macos", not(test)))]
fn delete_macos_keychain_origin_key(account: &str) -> Result<(), String> {
    match security_framework::passwords::delete_generic_password(MACOS_KEYCHAIN_SERVICE, account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => Ok(()),
        Err(error) => Err(format!(
            "retire origin key '{}' from macOS Keychain: {}",
            account, error
        )),
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

#[cfg(windows)]
fn protect_origin_key_for_current_user(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr::null;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| "origin key is too large to protect".to_string())?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let succeeded = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 {
        return Err(format!(
            "protect origin key for the current OS account: {}",
            std::io::Error::last_os_error()
        ));
    }
    if output.pbData.is_null()
        || output.cbData == 0
        || u64::from(output.cbData) > MAX_PROTECTED_ORIGIN_KEY_BYTES
    {
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        return Err("OS key protection returned an invalid payload".to_string());
    }
    let protected =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(protected)
}

#[cfg(windows)]
fn unprotect_origin_key_for_current_user(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| "protected origin key is too large".to_string())?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let succeeded = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 {
        return Err(format!(
            "unprotect origin key for the current OS account: {}",
            std::io::Error::last_os_error()
        ));
    }
    if output.pbData.is_null() || u64::from(output.cbData) != ORIGIN_KEY_BYTES {
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        return Err("OS key protection returned an invalid origin key".to_string());
    }
    let key = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(key)
}

fn collect_host_ids(value: &Value, host_ids: &mut BTreeSet<HostId>) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if let Some(value) = object.get("host_id") {
                let host_id = value
                    .as_str()
                    .ok_or_else(|| "local host_id is not a string".to_string())?;
                host_ids.insert(HostId::parse(host_id)?);
            }
            for (key, value) in object {
                if key != "host_id" {
                    collect_host_ids(value, host_ids)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_host_ids(value, host_ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn inject_host_id_into_records(value: &mut Value, host_id: &HostId) -> Result<bool, String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| "invalid local state document".to_string())?;
    if !object.contains_key("records") {
        object.insert("records".to_string(), Value::Array(Vec::new()));
    }
    let records = object
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "local state document has no records array".to_string())?;
    let mut migrated = false;
    for record in records {
        let record = record
            .as_object_mut()
            .ok_or_else(|| "local state document contains an invalid record".to_string())?;
        if !record.contains_key("host_id") {
            record.insert(
                "host_id".to_string(),
                Value::String(host_id.as_str().to_string()),
            );
            migrated = true;
        }
    }
    Ok(migrated)
}

fn decode_machine_project_state(
    mut value: Value,
    identity: &HostIdentity,
    config: &SyncConfigV3,
) -> Result<(MachineProjectState, bool), String> {
    let mut migrated = false;
    if value.get("schema").and_then(Value::as_u64) == Some(MACHINE_PROJECT_SCHEMA_V1 as u64) {
        let object = value
            .as_object_mut()
            .ok_or_else(|| "invalid legacy machine project state".to_string())?;
        object.insert("schema".to_string(), Value::from(MACHINE_PROJECT_SCHEMA_V2));
        if let Some(bindings) = object.get_mut("bindings").and_then(Value::as_array_mut) {
            for binding in bindings {
                let binding = binding
                    .as_object_mut()
                    .ok_or_else(|| "invalid legacy project binding".to_string())?;
                binding.insert(
                    "host_id".to_string(),
                    Value::String(identity.host_id.as_str().to_string()),
                );
            }
        }
        migrated = true;
    }
    let bindings: MachineProjectState =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    bindings.validate(config)?;
    Ok((bindings, migrated))
}

fn decode_setup_transaction(
    mut value: Value,
    identity: &HostIdentity,
) -> Result<(SetupTransaction, bool), String> {
    let mut migrated = false;
    if value.get("schema").and_then(Value::as_u64) == Some(SETUP_TRANSACTION_SCHEMA_V1 as u64) {
        let object = value
            .as_object_mut()
            .ok_or_else(|| "invalid legacy setup transaction".to_string())?;
        let replica_id = object
            .get("binding")
            .and_then(Value::as_object)
            .and_then(|binding| binding.get("replica_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "legacy setup transaction has no replica pin".to_string())?
            .to_string();
        object.insert(
            "schema".to_string(),
            Value::from(SETUP_TRANSACTION_SCHEMA_V2),
        );
        object.insert(
            "host_id".to_string(),
            Value::String(identity.host_id.as_str().to_string()),
        );
        object.insert("replica_id".to_string(), Value::String(replica_id));
        let binding = object
            .get_mut("binding")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "legacy setup transaction has no binding".to_string())?;
        binding.insert(
            "host_id".to_string(),
            Value::String(identity.host_id.as_str().to_string()),
        );
        migrated = true;
    }
    let transaction: SetupTransaction = serde_json::from_value(value)
        .map_err(|error| format!("parse setup transaction: {}", error))?;
    transaction.validate()?;
    Ok((transaction, migrated))
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
            "schema-3 path '{}' escapes '{}'",
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
                    "schema-3 path traverses symlink '{}'",
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

pub(crate) fn write_json_atomic<T: Serialize>(
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
        .write_all(&bytes)
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
    use crate::project_sync_v3::domain::{
        generated_named_id, BindingState, BundleId, BundleRecipe, ThreadId,
        ConversationMaterializationState, LocalConversationBinding,
        LocalProjectRegistration, LocalProviderProfileId, MaterializationRecord, OriginHostAlias,
        ProjectBinding, Provider, ProviderProfile, LOCAL_SCHEMA_V3,
        MACHINE_PROJECT_SCHEMA_V2,
    };
    use std::collections::BTreeMap;

    fn repository(temp: &tempfile::TempDir) -> V3Repository {
        V3Repository::from_home_dir(temp.path()).unwrap()
    }

    fn project() -> LocalProjectRegistration {
        LocalProjectRegistration {
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            display_name: "Project A".to_string(),
            local_alias: None,
            repository_fingerprint: None,
            recipe: BundleRecipe::default(),
            recipe_bases: BTreeMap::new(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn install_bound_project(
        repo: &V3Repository,
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
                provider: Provider::Codex,
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
                    Provider::Codex,
                    LocalProviderProfileId::parse("profile-a").unwrap(),
                )]),
                codex_home: None,
                claude_home: None,
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
    fn mallard_persistence_never_touches_schema2_files() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("sync_config.json");
        fs::write(&old, b"{\"schema\":2,\"keep\":true}\n").unwrap();
        let repo = repository(&temp);
        let saved = repo.save_config(SyncConfigV3::default()).unwrap();

        assert_eq!(saved.schema, LOCAL_SCHEMA_V3);
        assert_eq!(saved.revision, 1);
        assert_eq!(fs::read(&old).unwrap(), b"{\"schema\":2,\"keep\":true}\n");
        assert!(repo.config_path().starts_with(temp.path().join(".mallard")));
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
                provider: Provider::Codex,
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
                provider_thread_id: None,
                materialization_state: ConversationMaterializationState::StoredOnly,
                last_observed_file_sha256: None,
                last_observed_head_message_id: None,
                last_observed_lineage_digest: None,
                last_observed_semantic_digest: None,
                source_generation: None,
                source_manifest_sha256: None,
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
                    BTreeMap::from([(Provider::Codex, second_profile_id.clone())]);
                Ok(())
            })
            .unwrap_err();
        assert!(error.contains("scope is stale"));
        let after_profile_change = repo.load_bindings().unwrap();
        assert_eq!(
            after_profile_change.bindings[0].profile_ids[&Provider::Codex],
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
        let error = V3Repository::from_home_dir(temp.path()).unwrap_err();
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

        let key_path = repo.origin_key_path(&first.origin_key_ref);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&key_path).unwrap().len(), 32);
            assert_eq!(fs::metadata(&key_path).unwrap().mode() & 0o077, 0);
        }
        #[cfg(windows)]
        assert!(fs::metadata(&key_path).unwrap().len() > 32);
        let identity_json = fs::read_to_string(repo.host_identity_path()).unwrap();
        assert!(identity_json.contains(first.host_id.as_str()));
        assert!(!identity_json.contains("origin_key\":"));

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
            fs::read(repo.origin_key_path(&first.origin_key_ref)).unwrap(),
            fs::read(second_repo.origin_key_path(&other.origin_key_ref)).unwrap()
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
    fn startup_retires_only_unreferenced_well_formed_origin_keys() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let current = repo.load_or_create_host_identity().unwrap();
        let orphan = repo.prepare_host_identity_unlocked(None).unwrap();
        let orphan_path = repo.origin_key_path(&orphan.origin_key_ref);
        assert!(orphan_path.is_file());

        let (inspected, available) = repo.inspect_host_identity().unwrap();
        assert!(available);
        assert_eq!(inspected.host_id, current.host_id);
        assert!(repo.origin_key_path(&current.origin_key_ref).is_file());
        assert!(!orphan_path.exists());
    }

    #[test]
    fn failed_identity_publication_retires_its_prepared_key() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::create_dir(repo.host_identity_path()).unwrap();

        let error = repo.create_host_identity_unlocked(None).unwrap_err();
        assert!(
            error.contains("publish") || error.contains("regular file"),
            "{error}"
        );
        let keys = repo.origin_keys_dir();
        if keys.exists() {
            assert_eq!(fs::read_dir(keys).unwrap().count(), 0);
        }
    }

    #[test]
    fn private_key_fallback_requires_confirmed_protected_item_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let key_ref = OriginKeyRef::parse("originkey-fallback").unwrap();
        let marker = [0x5a_u8; ORIGIN_KEY_BYTES as usize];
        repo.ensure_directory(&repo.origin_keys_dir()).unwrap();
        repo.write_origin_key_file_unlocked(&key_ref, &marker)
            .unwrap();
        let key = OriginKey::generate().unwrap();

        let error = repo
            .write_private_origin_key_fallback_unlocked(
                &key_ref,
                &key,
                "protected store unavailable",
                || Err("cleanup denied".to_string()),
            )
            .unwrap_err();
        assert!(error.contains("could not be safely retired"), "{error}");
        assert_eq!(fs::read(repo.origin_key_path(&key_ref)).unwrap(), marker);

        repo.write_private_origin_key_fallback_unlocked(
            &key_ref,
            &key,
            "protected store unavailable",
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            fs::read(repo.origin_key_path(&key_ref)).unwrap(),
            key.as_bytes().as_slice()
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
    fn a_missing_origin_key_requires_explicit_new_host_registration() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        fs::remove_file(repo.origin_key_path(&original.origin_key_ref)).unwrap();

        let (inspected, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(inspected.host_id, original.host_id);
        assert!(!available);
        assert!(repo
            .load_or_create_host_identity()
            .unwrap_err()
            .contains("register as a new host"));

        let registered = repo
            .register_as_new_host(Some((&original.host_id, original.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, original.host_id);
        assert!(repo.inspect_host_identity().unwrap().1);
    }

    #[test]
    fn a_missing_identity_document_does_not_masquerade_as_first_run() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        repo.mutate_conversation_bindings(|_| Ok(())).unwrap();
        fs::remove_file(repo.host_identity_path()).unwrap();

        let (inspected, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(inspected.host_id, original.host_id);
        assert!(!available);
        assert!(repo
            .load_or_create_host_identity()
            .unwrap_err()
            .contains("identity document is unavailable"));

        let registered = repo
            .register_as_new_host(Some((&inspected.host_id, inspected.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, original.host_id);
        assert!(repo.inspect_host_identity().unwrap().1);
        assert_eq!(
            repo.load_conversation_bindings().unwrap().host_id,
            registered.host_id
        );
    }

    #[test]
    fn an_orphan_origin_key_prevents_silent_first_run_reinitialization() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        let original_key = repo.origin_key_path(&original.origin_key_ref);
        fs::remove_file(repo.host_identity_path()).unwrap();

        let (marker, available) = repo.inspect_host_identity().unwrap();
        assert!(!available);
        assert!(repo.is_unknown_identity_marker(&marker));
        assert!(original_key.is_file());
        assert!(repo
            .load_or_create_host_identity()
            .unwrap_err()
            .contains("register as a new host"));

        let registered = repo
            .register_as_new_host(Some((&marker.host_id, marker.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, original.host_id);
        assert!(!original_key.exists());
        assert!(repo.inspect_host_identity().unwrap().1);
    }

    #[test]
    fn semantically_invalid_host_scoped_state_does_not_masquerade_as_first_run() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::write(repo.conversation_bindings_path(), b"{}\n").unwrap();

        let (marker, available) = repo.inspect_host_identity().unwrap();
        assert!(!available);
        assert!(repo.is_unknown_identity_marker(&marker));
        assert!(!repo.origin_keys_dir().exists());
        assert!(repo
            .load_or_create_host_identity()
            .unwrap_err()
            .contains("register as a new host"));
    }

    #[test]
    fn valid_hostless_machine_schema_one_remains_a_first_run_migration() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let legacy = serde_json::json!({
            "schema": MACHINE_PROJECT_SCHEMA_V1,
            "revision": 0,
            "profiles": [],
            "bindings": []
        });
        write_json_atomic(
            repo.root(),
            &repo.machine_projects_path(),
            &legacy,
            MAX_BINDINGS_BYTES,
        )
        .unwrap();

        let (identity, available) = repo.inspect_host_identity().unwrap();
        assert!(available);
        assert!(!repo.is_unknown_identity_marker(&identity));
        assert_eq!(
            repo.load_bindings().unwrap().schema,
            MACHINE_PROJECT_SCHEMA_V2
        );
    }

    #[cfg(unix)]
    #[test]
    fn origin_key_with_broader_permissions_is_treated_as_unavailable() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let key_path = repo.origin_key_path(&identity.origin_key_ref);
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o640)).unwrap();

        let (inspected, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(inspected.host_id, identity.host_id);
        assert!(!available);
        assert!(repo
            .load_or_create_host_identity()
            .unwrap_err()
            .contains("unsafe permissions"));
    }

    #[test]
    fn same_length_origin_key_replacement_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let key_path = repo.origin_key_path(&identity.origin_key_ref);
        fs::write(&key_path, [0_u8; 32]).unwrap();

        let (inspected, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(inspected.host_id, identity.host_id);
        assert!(!available);
        assert!(repo
            .load_or_create_host_identity()
            .unwrap_err()
            .contains("does not match this host identity"));
    }

    #[test]
    fn schema_one_setup_transaction_migrates_with_current_host_and_replica_pins() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let draft_id = SetupDraftId::parse("draft-a").unwrap();
        let path = repo.setup_transaction_path(&draft_id);
        repo.ensure_directory(path.parent().unwrap()).unwrap();
        let legacy = serde_json::json!({
            "schema": 1,
            "draft_id": "draft-a",
            "draft_revision": 3,
            "created_at": 1,
            "profiles": [],
            "project": {
                "local_project_id": "project-a",
                "bundle_id": "0123456789abcdef0123456789abcdef",
                "display_name": "Project A",
                "recipe": { "schema_version": 1, "revision": 0, "entries": {} },
                "recipe_bases": {},
                "revision": 0,
                "created_at": 1,
                "updated_at": 1
            },
            "links": [],
            "binding": {
                "replica_id": "replica-legacy",
                "local_project_id": "project-a",
                "bundle_id": "0123456789abcdef0123456789abcdef",
                "project_root": project_root.to_string_lossy(),
                "canonical_project_root": project_root.to_string_lossy(),
                "profile_ids": {},
                "state": "active",
                "revision": 0,
                "updated_at": 1
            }
        });
        write_json_atomic(repo.root(), &path, &legacy, MAX_SETUP_TRANSACTION_BYTES).unwrap();

        let (transactions, warnings) = repo.list_setup_transactions().unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].schema, SETUP_TRANSACTION_SCHEMA_V2);
        assert_eq!(transactions[0].host_id, identity.host_id);
        assert_eq!(transactions[0].binding.host_id, identity.host_id);
        assert_eq!(transactions[0].replica_id.as_str(), "replica-legacy");
        assert_eq!(
            transactions[0].binding.replica_id.as_str(),
            "replica-legacy"
        );

        let migrated: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            migrated.get("schema").and_then(Value::as_u64),
            Some(SETUP_TRANSACTION_SCHEMA_V2 as u64)
        );
    }

    #[test]
    fn rotation_pins_legacy_setup_transactions_to_the_previous_host() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let draft_id = SetupDraftId::parse("draft-a").unwrap();
        let path = repo.setup_transaction_path(&draft_id);
        repo.ensure_directory(path.parent().unwrap()).unwrap();
        let legacy = serde_json::json!({
            "schema": 1,
            "draft_id": "draft-a",
            "draft_revision": 3,
            "created_at": 1,
            "profiles": [],
            "project": {
                "local_project_id": "project-a",
                "bundle_id": "0123456789abcdef0123456789abcdef",
                "display_name": "Project A",
                "recipe": { "schema_version": 1, "revision": 0, "entries": {} },
                "recipe_bases": {},
                "revision": 0,
                "created_at": 1,
                "updated_at": 1
            },
            "links": [],
            "binding": {
                "replica_id": "replica-legacy",
                "local_project_id": "project-a",
                "bundle_id": "0123456789abcdef0123456789abcdef",
                "project_root": project_root.to_string_lossy(),
                "canonical_project_root": project_root.to_string_lossy(),
                "profile_ids": {},
                "state": "active",
                "revision": 0,
                "updated_at": 1
            }
        });
        write_json_atomic(repo.root(), &path, &legacy, MAX_SETUP_TRANSACTION_BYTES).unwrap();

        let rotated = repo
            .rotate_host_identity(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        let migrated: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            migrated.get("schema").and_then(Value::as_u64),
            Some(SETUP_TRANSACTION_SCHEMA_V2 as u64)
        );
        assert_eq!(
            migrated.get("host_id").and_then(Value::as_str),
            Some(previous.host_id.as_str())
        );
        assert_ne!(
            migrated.get("host_id").and_then(Value::as_str),
            Some(rotated.host_id.as_str())
        );
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

        let registered = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
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
    fn stale_host_rotation_journal_cannot_replace_an_unrelated_identity() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
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
        assert!(repo.host_rotation_transaction_is_visible_unlocked(&transaction));
        let mut replacement = transaction.clone();
        replacement.created_at = replacement.created_at.saturating_add(1);
        assert!(!repo.host_rotation_transaction_is_visible_unlocked(&replacement));

        let unrelated = repo.prepare_host_identity_unlocked(None).unwrap();
        write_json_atomic(
            repo.root(),
            &repo.host_identity_path(),
            &unrelated,
            MAX_HOST_IDENTITY_BYTES,
        )
        .unwrap();

        let error = repo.load_or_create_host_identity().unwrap_err();
        assert!(error.contains("unrelated host identity"), "{error}");
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(unrelated)
        );
        assert!(repo.host_rotation_path().is_file());
    }

    #[test]
    fn rotation_requires_a_fresh_key_reference_and_bound_verifier() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let mut next = repo.prepare_host_identity_unlocked(None).unwrap();
        next.origin_key_ref = previous.origin_key_ref.clone();
        next.origin_key_verifier = None;
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id,
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref,
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine::default(),
            created_at: 1,
        };

        assert!(repo
            .validate_host_rotation_payload_unlocked(&transaction, &repo.load_config().unwrap())
            .unwrap_err()
            .contains("invalid host rotation"));
    }

    #[test]
    fn mixed_secondary_host_evidence_cannot_anchor_a_stale_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
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
        fs::remove_file(repo.host_identity_path()).unwrap();

        let plans = repo.root().join("restore_plans");
        repo.ensure_directory(&plans).unwrap();
        write_json_atomic(
            repo.root(),
            &plans.join("previous.json"),
            &serde_json::json!({ "host_id": previous.host_id }),
            MAX_RESTORE_PLAN_BYTES,
        )
        .unwrap();
        write_json_atomic(
            repo.root(),
            &plans.join("unrelated.json"),
            &serde_json::json!({
                "host_id": "ffeeddccbbaa99887766554433221100"
            }),
            MAX_RESTORE_PLAN_BYTES,
        )
        .unwrap();

        let error = repo.load_or_create_host_identity().unwrap_err();
        assert!(error.contains("does not match local host-scoped state"));
        assert!(!repo.host_identity_path().exists());
        assert!(repo.host_rotation_path().is_file());
        assert!(!repo.machine_projects_path().exists());

        let (marker, available) = repo.inspect_host_identity().unwrap();
        assert!(!available);
        assert!(repo.is_unknown_identity_marker(&marker));
        assert_ne!(marker.host_id, next.host_id);
    }

    #[test]
    fn explicit_registration_salvages_a_partially_committed_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let previous_replica = install_bound_project(&repo, &temp, &previous);
        repo.mutate_conversation_bindings(|conversations| {
            conversations.bindings.push(LocalConversationBinding {
                host_id: previous.host_id.clone(),
                replica_id: previous_replica.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: ThreadId::parse("group-a.variant-a").unwrap(),
                provider_thread_id: Some("thread-a".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: None,
                last_observed_head_message_id: None,
                last_observed_lineage_digest: None,
                last_observed_semantic_digest: None,
                source_generation: None,
                source_manifest_sha256: None,
                revision: 0,
                created_at: u64::MAX - 1,
                updated_at: u64::MAX - 1,
            });
            Ok(())
        })
        .unwrap();

        let config = repo.load_config().unwrap();
        let mut machine = repo.load_bindings().unwrap();
        let mut conversations = repo.load_conversation_bindings().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let next_replica = ReplicaId::parse("replica-next").unwrap();
        machine.revision = machine.revision.saturating_add(1);
        machine.bindings[0].host_id = next.host_id.clone();
        machine.bindings[0].replica_id = next_replica.clone();
        machine.bindings[0].state = BindingState::Detached;
        machine.bindings[0].revision = machine.bindings[0].revision.saturating_add(1);
        machine.validate(&config).unwrap();
        conversations.host_id = next.host_id.clone();
        conversations.revision = conversations.revision.saturating_add(1);
        conversations.bindings[0].host_id = next.host_id.clone();
        conversations.bindings[0].replica_id = next_replica;
        conversations.bindings[0].materialization_state = ConversationMaterializationState::Pending;
        conversations.validate(&next, &machine, &config).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
            next_identity: next.clone(),
            machine_projects: machine.clone(),
            conversation_bindings: conversations,
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
        // Simulate the first document commit, followed by loss of the next
        // protected key before recovery can finish the transaction.
        write_json_atomic(
            repo.root(),
            &repo.machine_projects_path(),
            &machine,
            MAX_BINDINGS_BYTES,
        )
        .unwrap();
        fs::remove_file(repo.origin_key_path(&next.origin_key_ref)).unwrap();

        let (inspected, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(inspected.host_id, previous.host_id);
        assert!(!available);
        let registered = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        let after_machine = repo.load_bindings().unwrap();
        let after_conversations = repo.load_conversation_bindings().unwrap();
        assert_eq!(after_conversations.bindings.len(), 1);
        assert_eq!(after_conversations.bindings[0].host_id, registered.host_id);
        assert_eq!(
            after_conversations.bindings[0].replica_id,
            after_machine.bindings[0].replica_id
        );
        assert_eq!(
            after_conversations.bindings[0].thread_id.as_str(),
            "group-a.variant-a"
        );
        assert_eq!(
            after_conversations.bindings[0]
                .provider_thread_id
                .as_deref(),
            Some("thread-a")
        );
        assert_eq!(after_conversations.bindings[0].updated_at, u64::MAX - 1);
    }

    #[test]
    fn explicit_registration_salvages_valid_conversation_identity_beside_a_stale_association() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let replica = install_bound_project(&repo, &temp, &previous);
        repo.mutate_conversation_bindings(|conversations| {
            conversations.bindings.push(LocalConversationBinding {
                host_id: previous.host_id.clone(),
                replica_id: replica.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: ThreadId::parse("thread-valid").unwrap(),
                provider_thread_id: Some("thread-valid".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: None,
                last_observed_head_message_id: None,
                last_observed_lineage_digest: None,
                last_observed_semantic_digest: None,
                source_generation: None,
                source_manifest_sha256: None,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        let mut damaged = repo.load_conversation_bindings().unwrap();
        let mut stale = damaged.bindings[0].clone();
        stale.provider_profile_id = LocalProviderProfileId::parse("profile-missing").unwrap();
        stale.thread_id = ThreadId::parse("thread-stale").unwrap();
        stale.provider_thread_id = Some("thread-stale".to_string());
        damaged.bindings.push(stale);
        write_json_atomic(
            repo.root(),
            &repo.conversation_bindings_path(),
            &damaged,
            MAX_CONVERSATION_BINDINGS_BYTES,
        )
        .unwrap();

        let next = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        let after = repo.load_conversation_bindings().unwrap();
        assert_eq!(after.bindings.len(), 1);
        assert_eq!(after.bindings[0].host_id, next.host_id);
        assert_eq!(after.bindings[0].thread_id.as_str(), "thread-valid");
        assert_eq!(
            after.bindings[0].provider_thread_id.as_deref(),
            Some("thread-valid")
        );
        assert!(fs::read_dir(repo.root().join("host_recovery_quarantine"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("conversation-bindings-")));
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
        assert_eq!(fs::read_dir(repo.origin_keys_dir()).unwrap().count(), 1);
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
        assert_eq!(fs::read_dir(repo.origin_keys_dir()).unwrap().count(), 1);
    }

    #[test]
    fn committed_next_identity_stays_available_while_key_cleanup_is_pending() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let mut config = repo.load_config().unwrap();
        config.projects.push(project());
        repo.save_config(config).unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let mut journaled_materializations = Materializations::default();
        journaled_materializations
            .records
            .push(MaterializationRecord {
                materialization_id: super::super::domain::MaterializationId::parse(
                    "materialization-cleanup",
                )
                .unwrap(),
                plan_id: PlanId::parse("plan-cleanup").unwrap(),
                host_id: previous.host_id.clone(),
                replica_id: ReplicaId::parse("replica-cleanup").unwrap(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                storage_id: super::super::domain::StorageId::parse("storage-cleanup").unwrap(),
                bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
                generation: 1,
                commit_id: "commit-cleanup".to_string(),
                manifest_sha256: "ab".repeat(32),
                binding_revision: 0,
                status: super::super::domain::MaterializationStatus::Detached,
                applied_at: 1,
                receipts: Vec::new(),
            });
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: journaled_materializations,
            quarantine: HostRotationQuarantine::default(),
            created_at: next.updated_at,
        };
        write_json_atomic(
            repo.root(),
            &repo.host_rotation_path(),
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        )
        .unwrap();

        // A directory at the retired key path deterministically models a
        // protected-store cleanup failure without weakening the live next key.
        let previous_key_path = repo.origin_key_path(&previous.origin_key_ref);
        fs::remove_file(&previous_key_path).unwrap();
        fs::create_dir(&previous_key_path).unwrap();

        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status, next);
        assert!(available);
        assert!(repo.host_rotation_path().is_file());
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(next.clone())
        );

        // A valid config edit can make the journal's already-applied snapshot
        // stale. Cleanup-only recovery must rely only on immutable identity/key
        // anchors and must not replay or revalidate that old snapshot.
        let mut changed_config = repo.load_config().unwrap();
        changed_config.projects[0].bundle_id =
            BundleId::parse("fedcba9876543210fedcba9876543210").unwrap();
        repo.save_config(changed_config).unwrap();

        // Cleanup retries must never replay the journal's document snapshots
        // over valid work completed after the next identity became current.
        repo.mutate_bindings(|_, _| Ok(())).unwrap();
        let machine_revision = repo.load_bindings().unwrap().revision;
        assert_eq!(machine_revision, 1);
        assert_eq!(repo.load_bindings().unwrap().revision, machine_revision);
        let renamed = repo
            .update_host_display_name(
                &next.host_id,
                next.revision,
                Some("Current host".to_string()),
            )
            .unwrap();
        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status, renamed);
        assert!(available);

        let error = repo
            .register_as_new_host(Some((&renamed.host_id, renamed.revision)), None)
            .unwrap_err();
        assert!(error.contains("cleanup is still pending"), "{error}");
        assert_eq!(
            repo.load_host_identity_document_unlocked().unwrap(),
            Some(renamed.clone())
        );

        fs::remove_dir(previous_key_path).unwrap();
        assert_eq!(repo.load_or_create_host_identity().unwrap(), renamed);
        assert!(!repo.host_rotation_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_previous_key_keeps_rotation_cleanup_journaled() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let previous_key = repo.origin_key_path(&previous.origin_key_ref);
        let outside_link = temp.path().join("linked-origin-key");
        fs::hard_link(&previous_key, &outside_link).unwrap();

        let next = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        assert_ne!(next.host_id, previous.host_id);
        assert!(previous_key.is_file());
        assert!(outside_link.is_file());
        assert!(repo.host_rotation_path().is_file());

        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status, next);
        assert!(available);
        assert!(repo.host_rotation_path().is_file());
        let error = repo
            .register_as_new_host(Some((&status.host_id, status.revision)), None)
            .unwrap_err();
        assert!(error.contains("cleanup is still pending"), "{error}");
    }

    #[test]
    fn explicit_registration_supersedes_a_rotation_whose_next_key_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
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
        fs::remove_file(repo.origin_key_path(&next.origin_key_ref)).unwrap();

        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status.host_id, previous.host_id);
        assert!(!available);
        let registered = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert_ne!(registered.host_id, next.host_id);
        assert!(!repo.host_rotation_path().exists());
        assert!(repo.inspect_host_identity().unwrap().1);
    }

    #[test]
    fn explicit_registration_quarantines_an_invalid_rotation_journal() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let previous_key = repo.origin_key_path(&previous.origin_key_ref);
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            // The identity anchor is trustworthy enough to identify the
            // recovery request, but the payload itself must never be replayed.
            schema: 99,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
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

        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status.host_id, previous.host_id);
        assert!(!available);
        let registered = repo
            .register_as_new_host(Some((&status.host_id, status.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert!(!repo.host_rotation_path().exists());
        assert!(!previous_key.exists());
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
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
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
        let result = repo.register_as_new_host(Some((&previous.host_id, previous.revision)), None);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let error = result.unwrap_err();
        assert!(error.contains("open"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), preserved);
        assert!(repo.host_rotation_transaction_is_visible_unlocked(&transaction));
        assert!(!repo.root().join("host_recovery_quarantine").exists());
    }

    #[test]
    fn explicit_registration_quarantines_unreadable_rotation_and_setup_journals() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        fs::write(repo.host_rotation_path(), b"{not-json").unwrap();
        let setup = repo.root().join("setup_transactions").join("broken.json");
        repo.ensure_directory(setup.parent().unwrap()).unwrap();
        fs::write(&setup, b"{also-not-json").unwrap();

        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status.host_id, previous.host_id);
        assert!(!available);
        let registered = repo
            .register_as_new_host(Some((&status.host_id, status.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert!(!setup.exists());
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
    fn explicit_registration_journals_replacement_before_quarantining_live_state() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        fs::remove_file(repo.origin_key_path(&previous.origin_key_ref)).unwrap();
        fs::write(repo.machine_projects_path(), b"{broken-machine").unwrap();
        fs::write(repo.materializations_path(), b"{broken-materializations").unwrap();
        fs::write(
            repo.dependency_applications_path(),
            b"{broken-dependency-receipts",
        )
        .unwrap();

        let error = repo
            .register_as_new_host(
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

        let registered = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert!(repo.load_bindings().unwrap().bindings.is_empty());
        assert!(repo.load_materializations().unwrap().records.is_empty());
        assert!(repo
            .load_dependency_applications()
            .unwrap()
            .records
            .is_empty());
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
        assert!(names
            .iter()
            .any(|name| name.starts_with("dependency-applications-")));
    }

    #[test]
    fn recovery_replays_journaled_quarantine_before_retiring_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine {
                machine_projects: true,
                conversation_bindings: true,
                materializations: true,
                dependency_applications: true,
            },
            created_at: 1,
        };
        repo.validate_host_rotation_unlocked(&transaction, &repo.load_config().unwrap())
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
            (
                repo.dependency_applications_path(),
                b"{broken-dependency-receipts".as_slice(),
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
        assert!(!repo.dependency_applications_path().exists());
        assert!(!repo.host_rotation_path().exists());
        for category in [
            "machine-projects",
            "conversation-bindings",
            "materializations",
            "dependency-applications",
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
        let previous = repo.load_or_create_host_identity().unwrap();
        let next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref,
            next_identity: next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine {
                dependency_applications: true,
                ..HostRotationQuarantine::default()
            },
            created_at: 1,
        };

        let error = repo
            .quarantine_host_rotation_file_unlocked(
                &transaction,
                &repo.dependency_applications_path(),
                "dependency-applications",
            )
            .unwrap_err();
        assert!(error.contains("confirm preserved"), "{error}");
    }

    #[test]
    fn explicit_registration_settles_salvaged_quarantine_before_rejournaling() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let abandoned_next = repo.prepare_host_identity_unlocked(None).unwrap();
        let transaction = HostRotationTransaction {
            schema: 1,
            previous_host_id: previous.host_id.clone(),
            previous_identity_revision: previous.revision,
            previous_origin_key_ref: previous.origin_key_ref.clone(),
            next_identity: abandoned_next.clone(),
            machine_projects: MachineProjectState::default(),
            conversation_bindings: ConversationBindings::empty(abandoned_next.host_id.clone()),
            materializations: Materializations::default(),
            quarantine: HostRotationQuarantine {
                dependency_applications: true,
                ..HostRotationQuarantine::default()
            },
            created_at: 1,
        };
        fs::write(
            repo.dependency_applications_path(),
            b"{damaged-dependency-applications",
        )
        .unwrap();
        write_json_atomic(
            repo.root(),
            &repo.host_rotation_path(),
            &transaction,
            MAX_HOST_ROTATION_BYTES,
        )
        .unwrap();
        repo.quarantine_host_rotation_file_unlocked(
            &transaction,
            &repo.dependency_applications_path(),
            "dependency-applications",
        )
        .unwrap();
        let preserved =
            repo.host_rotation_quarantine_destination(&transaction, "dependency-applications");
        assert!(preserved.is_file());
        fs::remove_file(repo.origin_key_path(&abandoned_next.origin_key_ref)).unwrap();

        let registered = repo
            .register_as_new_host(Some((&previous.host_id, previous.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, previous.host_id);
        assert_ne!(registered.host_id, abandoned_next.host_id);
        assert!(preserved.is_file());
        assert!(!repo.host_rotation_path().exists());
        assert!(repo.inspect_host_identity().unwrap().1);
    }

    #[test]
    fn unreadable_partial_rotation_preserves_conversation_identity_as_pending() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let previous = repo.load_or_create_host_identity().unwrap();
        let previous_replica = install_bound_project(&repo, &temp, &previous);
        repo.mutate_conversation_bindings(|conversations| {
            conversations.bindings.push(LocalConversationBinding {
                host_id: previous.host_id.clone(),
                replica_id: previous_replica.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: ThreadId::parse("group-a.variant-a").unwrap(),
                provider_thread_id: Some("thread-a".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some("ab".repeat(32)),
                last_observed_head_message_id: Some("message-a".to_string()),
                last_observed_lineage_digest: Some("cd".repeat(32)),
                last_observed_semantic_digest: Some("ef".repeat(32)),
                source_generation: Some(7),
                source_manifest_sha256: Some("12".repeat(32)),
                revision: 3,
                created_at: 1,
                updated_at: 2,
            });
            Ok(())
        })
        .unwrap();

        let config = repo.load_config().unwrap();
        let mut partial_machine = repo.load_bindings().unwrap();
        let partial_host = HostId::generate().unwrap();
        let partial_replica = ReplicaId::parse("replica-partial").unwrap();
        partial_machine.revision = partial_machine.revision.saturating_add(1);
        partial_machine.bindings[0].host_id = partial_host;
        partial_machine.bindings[0].replica_id = partial_replica;
        partial_machine.bindings[0].state = BindingState::Detached;
        partial_machine.bindings[0].revision =
            partial_machine.bindings[0].revision.saturating_add(1);
        partial_machine.validate(&config).unwrap();
        write_json_atomic(
            repo.root(),
            &repo.machine_projects_path(),
            &partial_machine,
            MAX_BINDINGS_BYTES,
        )
        .unwrap();
        fs::write(repo.host_rotation_path(), b"{unreadable-journal").unwrap();

        let (status, available) = repo.inspect_host_identity().unwrap();
        assert_eq!(status.host_id, previous.host_id);
        assert!(!available);
        let registered = repo
            .register_as_new_host(Some((&status.host_id, status.revision)), None)
            .unwrap();
        let machine = repo.load_bindings().unwrap();
        let conversations = repo.load_conversation_bindings().unwrap();
        assert_eq!(conversations.bindings.len(), 1);
        let binding = &conversations.bindings[0];
        assert_eq!(binding.host_id, registered.host_id);
        assert_eq!(binding.replica_id, machine.bindings[0].replica_id);
        assert_eq!(binding.thread_id.as_str(), "group-a.variant-a");
        assert_eq!(binding.provider_thread_id.as_deref(), Some("thread-a"));
        assert_eq!(
            binding.materialization_state,
            ConversationMaterializationState::Pending
        );
        assert_eq!(binding.last_observed_semantic_digest, Some("ef".repeat(32)));
    }

    #[test]
    fn ambiguous_retained_hosts_use_an_explicit_unknown_identity_recovery_marker() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let plans = repo.root().join("restore_plans");
        repo.ensure_directory(&plans).unwrap();
        write_json_atomic(
            repo.root(),
            &plans.join("one.json"),
            &serde_json::json!({
                "host_id": "00112233445566778899aabbccddeeff"
            }),
            MAX_RESTORE_PLAN_BYTES,
        )
        .unwrap();
        write_json_atomic(
            repo.root(),
            &plans.join("two.json"),
            &serde_json::json!({
                "host_id": "ffeeddccbbaa99887766554433221100"
            }),
            MAX_RESTORE_PLAN_BYTES,
        )
        .unwrap();

        let (marker, available) = repo.inspect_host_identity().unwrap();
        assert!(!available);
        assert!(repo.is_unknown_identity_marker(&marker));
        let registered = repo
            .register_as_new_host(Some((&marker.host_id, marker.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, marker.host_id);
        assert!(!repo.is_unknown_identity_marker(&registered));
        assert!(repo.inspect_host_identity().unwrap().1);
    }

    #[test]
    fn a_malformed_identity_is_preserved_and_can_be_explicitly_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let original = repo.load_or_create_host_identity().unwrap();
        let original_key = repo.origin_key_path(&original.origin_key_ref);
        fs::write(repo.host_identity_path(), b"{not-json").unwrap();

        let (marker, available) = repo.inspect_host_identity().unwrap();
        assert!(!available);
        assert!(repo.is_unknown_identity_marker(&marker));
        assert!(original_key.is_file());
        // Repeated status checks must not reinterpret a recoverable key as an
        // orphan while the explicit-recovery marker is live.
        assert!(!repo.inspect_host_identity().unwrap().1);
        assert!(original_key.is_file());
        let registered = repo
            .register_as_new_host(Some((&marker.host_id, marker.revision)), None)
            .unwrap();
        assert_ne!(registered.host_id, original.host_id);
        assert!(!original_key.exists());
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
        let result = repo.inspect_host_identity();
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

    #[test]
    fn machine_schema_one_migrates_once_and_rotation_detaches_fresh_replicas() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let identity = repo.load_or_create_host_identity().unwrap();
        let project_root = temp.path().join("project");
        let profile_root = temp.path().join("codex");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&profile_root).unwrap();

        let mut config = repo.load_config().unwrap();
        config.projects.push(LocalProjectRegistration {
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            display_name: "Project A".to_string(),
            local_alias: None,
            repository_fingerprint: None,
            recipe: BundleRecipe::default(),
            recipe_bases: BTreeMap::new(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        });
        repo.save_config(config).unwrap();

        let legacy = serde_json::json!({
            "schema": 1,
            "revision": 4,
            "profiles": [{
                "profile_id": "profile-a",
                "provider": "codex",
                "display_name": "Codex",
                "path": profile_root.to_string_lossy(),
                "canonical_path": profile_root.to_string_lossy(),
                "revision": 0,
                "created_at": 1,
                "updated_at": 1
            }],
            "bindings": [{
                "replica_id": "replica-old",
                "local_project_id": "project-a",
                "bundle_id": "0123456789abcdef0123456789abcdef",
                "project_root": project_root.to_string_lossy(),
                "canonical_project_root": project_root.to_string_lossy(),
                "profile_ids": { "codex": "profile-a" },
                "state": "active",
                "revision": 2,
                "updated_at": 1
            }]
        });
        write_json_atomic(
            repo.root(),
            &repo.machine_projects_path(),
            &legacy,
            MAX_BINDINGS_BYTES,
        )
        .unwrap();

        let migrated = repo.load_bindings().unwrap();
        assert_eq!(migrated.schema, MACHINE_PROJECT_SCHEMA_V2);
        assert_eq!(migrated.revision, 4);
        assert_eq!(migrated.bindings[0].host_id, identity.host_id);
        let old_replica = migrated.bindings[0].replica_id.clone();
        repo.mutate_conversation_bindings(|conversations| {
            conversations.bindings.push(LocalConversationBinding {
                host_id: identity.host_id.clone(),
                replica_id: old_replica.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
                thread_id: ThreadId::parse("group-a.variant-a").unwrap(),
                provider_thread_id: Some("thread-a".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: None,
                last_observed_head_message_id: None,
                last_observed_lineage_digest: None,
                last_observed_semantic_digest: None,
                source_generation: None,
                source_manifest_sha256: None,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        repo.mutate_materializations(|_, materializations| {
            materializations.records.push(MaterializationRecord {
                materialization_id: super::super::domain::MaterializationId::parse(
                    "materialization-a",
                )
                .unwrap(),
                plan_id: PlanId::parse("plan-a").unwrap(),
                host_id: identity.host_id.clone(),
                replica_id: old_replica.clone(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                storage_id: super::super::domain::StorageId::parse("storage-a").unwrap(),
                bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
                generation: 1,
                commit_id: "commit-a".to_string(),
                manifest_sha256: "ab".repeat(32),
                binding_revision: 2,
                status: super::super::domain::MaterializationStatus::Complete,
                applied_at: 1,
                receipts: Vec::new(),
            });
            Ok(())
        })
        .unwrap();
        let rotated = repo
            .rotate_host_identity(Some((&identity.host_id, identity.revision)), None)
            .unwrap();
        let after = repo.load_bindings().unwrap();
        assert_eq!(after.bindings[0].host_id, rotated.host_id);
        assert_ne!(after.bindings[0].replica_id, old_replica);
        assert_eq!(after.bindings[0].state, BindingState::Detached);
        let conversations = repo.load_conversation_bindings().unwrap();
        assert_eq!(conversations.bindings.len(), 1);
        assert_eq!(conversations.bindings[0].host_id, rotated.host_id);
        assert_eq!(
            conversations.bindings[0].replica_id,
            after.bindings[0].replica_id
        );
        assert_eq!(
            conversations.bindings[0].materialization_state,
            ConversationMaterializationState::Pending
        );
        assert_eq!(
            conversations.bindings[0].thread_id.as_str(),
            "group-a.variant-a"
        );
        assert_eq!(
            conversations.bindings[0].provider_thread_id.as_deref(),
            Some("thread-a")
        );
        let materializations = repo.load_materializations().unwrap();
        assert_eq!(materializations.records.len(), 1);
        assert_eq!(materializations.records[0].host_id, identity.host_id);
        assert_eq!(materializations.records[0].replica_id, old_replica);
        assert_eq!(
            materializations.records[0].status,
            super::super::domain::MaterializationStatus::Detached
        );
    }
}
