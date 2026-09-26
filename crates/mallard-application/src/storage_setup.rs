//! Provider-neutral storage setup shared by the desktop and CLI adapters.
//!
//! Listing, drafts, connection tests, commits, renames, removals, and
//! verification are common flows. Every provider-specific decision — typed
//! settings, secrets, probes, continuity coordinates, and the setup
//! assistant — is resolved through the compiled
//! [`StorageProviderRegistry`]. No function here names a storage provider.

use mallard_domain::{
    generated_named_id, BoundedConfigObject, SecretKey, SecretMap, SecretOperation, SecretValue,
    StorageConfig, StorageId, StorageProviderConfig, StorageProviderKey, StorageSetupDraft,
    StorageSetupDraftId, StorageSetupDraftSource, SyncConfig, STORAGE_SETUP_DRAFT_SCHEMA,
};
use mallard_infrastructure::persistence::{
    ConfigMutation, ConfigTransactionError, HostOperationGuard, LocalStateRepository,
};
use mallard_infrastructure::storage_core::{ConfiguredObjectStore, ConnectionHealth};
use mallard_infrastructure::storage_providers::{
    StorageAdapterError, StorageAdapterErrorKind, StorageProviderAdapter,
    StorageProviderDescriptor, StorageProviderRegistry,
    StorageSummaryField,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle;
use zeroize::Zeroizing;

const MAX_STORAGE_NAME_BYTES: usize = 1_024;

/// A secret-bearing string that zeroizes its allocation on drop. It has no
/// `Debug` or `Serialize` implementation, so adapters cannot accidentally log
/// or emit it through ordinary derived output.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn into_secret_value(self) -> Result<SecretValue, StorageSetupError> {
        SecretValue::parse(self.0.as_str()).map_err(|_| {
            StorageSetupError::invalid("storage_secret_invalid", "The secret value is invalid.")
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageSetupErrorKind {
    InvalidInput,
    Conflict,
    RemoteFailure,
    PersistenceFailure,
}

impl StorageSetupErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::InvalidInput => 2,
            Self::Conflict => 3,
            Self::RemoteFailure => 4,
            Self::PersistenceFailure => 5,
        }
    }
}

/// A typed, sanitized setup error. Messages are fixed text chosen by this
/// module or by an adapter; settings, secrets, destinations, SDK output, and
/// filesystem values are never retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageSetupError {
    kind: StorageSetupErrorKind,
    code: &'static str,
    message: String,
}

impl StorageSetupError {
    pub(crate) fn fixed(kind: StorageSetupErrorKind, code: &'static str, message: &str) -> Self {
        Self {
            kind,
            code,
            message: message.to_string(),
        }
    }

    pub(crate) fn invalid(code: &'static str, message: &str) -> Self {
        Self::fixed(StorageSetupErrorKind::InvalidInput, code, message)
    }

    pub(crate) fn conflict(code: &'static str, message: &str) -> Self {
        Self::fixed(StorageSetupErrorKind::Conflict, code, message)
    }

    pub(crate) fn persistence() -> Self {
        Self::fixed(
            StorageSetupErrorKind::PersistenceFailure,
            "local_config_unavailable",
            "Mallard could not access its local storage configuration. If this command is running in a restricted environment, approve read/write access to Mallard's local state and run the same command again.",
        )
    }

    fn from_adapter(error: StorageAdapterError) -> Self {
        match error.kind() {
            StorageAdapterErrorKind::Invalid => Self::invalid(error.code(), error.message()),
            StorageAdapterErrorKind::Remote | StorageAdapterErrorKind::Internal => Self::fixed(
                StorageSetupErrorKind::RemoteFailure,
                error.code(),
                error.message(),
            ),
        }
    }

    pub fn kind(&self) -> StorageSetupErrorKind {
        self.kind
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for StorageSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StorageSetupError {}

/// One configured storage as every listing shows it: identity, provider
/// coordinate, availability under this build's registry, the adapter's
/// redacted summary, and what the open transport last observed about its
/// credential. Never a secret, never a full provider config.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StorageSummary {
    pub storage_id: StorageId,
    pub name: String,
    pub provider: StorageProviderKey,
    pub config_version: u32,
    pub provider_label: String,
    pub icon: String,
    pub available: bool,
    pub summary: Vec<StorageSummaryField>,
    /// The safe settings as saved. The storage page shows them read-only:
    /// a destination is fixed for the life of a storage.
    pub settings: BoundedConfigObject,
    /// Which declared secrets the storage holds. Values never travel with a
    /// listing; `reveal_storage_secret` returns one on an explicit request.
    pub secrets: Vec<StorageSecretPresence>,
    /// Runtime connection health from the process-shared store, absent
    /// until a request has been made in this process. Never persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionHealth>,
}

/// Presence of one secret on a saved storage.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StorageSecretPresence {
    pub secret_id: SecretKey,
    pub present: bool,
}

/// Every declared secret, present or not, then any stored key the
/// descriptor no longer declares (always present).
fn secret_presence(
    descriptor: Option<&StorageProviderDescriptor>,
    provider_config: &StorageProviderConfig,
) -> Vec<StorageSecretPresence> {
    let mut secrets = Vec::new();
    let mut listed = BTreeSet::new();
    if let Some(descriptor) = descriptor {
        for secret in &descriptor.secrets {
            listed.insert(secret.secret_id.clone());
            secrets.push(StorageSecretPresence {
                secret_id: secret.secret_id.clone(),
                present: provider_config.secrets.contains(&secret.secret_id),
            });
        }
    }
    for key in provider_config.secrets.keys() {
        if !listed.contains(key) {
            secrets.push(StorageSecretPresence {
                secret_id: key.clone(),
                present: true,
            });
        }
    }
    secrets
}

fn validated_registered_adapter<'a>(
    registry: &'a StorageProviderRegistry,
    storage: &StorageConfig,
) -> Result<Option<&'a dyn StorageProviderAdapter>, StorageSetupError> {
    storage.validate().map_err(|_| {
        StorageSetupError::invalid(
            "storage_configuration_invalid",
            "The storage configuration is invalid.",
        )
    })?;
    let Some(adapter) = registry.get(&storage.provider, storage.provider_config.version) else {
        return Ok(None);
    };
    adapter
        .validate_config(&storage.provider_config)
        .map_err(StorageSetupError::from_adapter)?;
    Ok(Some(adapter))
}

pub fn storage_summary(
    registry: &StorageProviderRegistry,
    storage: &StorageConfig,
) -> Result<StorageSummary, StorageSetupError> {
    let adapter = validated_registered_adapter(registry, storage)?;
    let descriptor = adapter.map(StorageProviderAdapter::descriptor);
    let summary = match adapter {
        Some(adapter) => adapter.summary(&storage.provider_config).map_err(|_| {
            StorageSetupError::invalid(
                "storage_configuration_invalid",
                "The storage configuration is invalid.",
            )
        })?,
        None => Vec::new(),
    };
    Ok(StorageSummary {
        storage_id: storage.id.clone(),
        name: storage.name.clone(),
        provider: storage.provider.clone(),
        config_version: storage.provider_config.version,
        provider_label: descriptor
            .as_ref()
            .map(|descriptor| descriptor.display_name.clone())
            .unwrap_or_else(|| storage.provider.as_str().to_string()),
        icon: descriptor
            .as_ref()
            .map(|descriptor| descriptor.icon.clone())
            .unwrap_or_else(|| "storage".to_string()),
        available: adapter.is_some(),
        summary,
        settings: storage.provider_config.settings.clone(),
        secrets: secret_presence(descriptor.as_ref(), &storage.provider_config),
        connection: ConfiguredObjectStore::connection_health_for(storage),
    })
}

/// One saved secret, on an explicit request from the storage page's Show
/// or Copy. Only a secret the adapter declares can be read; listings,
/// drafts, and updates stay redacted. The caller logs the reveal, never
/// the value.
pub fn reveal_storage_secret(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    storage_id: &StorageId,
    secret_id: &SecretKey,
) -> Result<String, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    let storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(storage_not_found)?;
    let adapter = registry
        .require(&storage.provider, storage.provider_config.version)
        .map_err(|_| storage_unavailable())?;
    if !adapter
        .descriptor()
        .secrets
        .iter()
        .any(|secret| &secret.secret_id == secret_id)
    {
        return Err(StorageSetupError::invalid(
            "storage_secret_unknown",
            "The storage provider declares no secret with that identifier.",
        ));
    }
    storage
        .provider_config
        .secrets
        .get(secret_id)
        .map(|value| value.expose().to_string())
        .ok_or_else(|| {
            StorageSetupError::invalid(
                "storage_secret_missing",
                "This storage holds no value for that secret.",
            )
        })
}

pub fn list_storages(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
) -> Result<Vec<StorageSummary>, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    config
        .storages
        .iter()
        .map(|storage| storage_summary(registry, storage))
        .collect()
}

/// The machine-local directory a configured storage occupies, resolved
/// through its adapter. Unavailable providers report no local root.
pub fn storage_local_root(
    registry: &StorageProviderRegistry,
    storage: &StorageConfig,
) -> Result<Option<PathBuf>, String> {
    match validated_registered_adapter(registry, storage).map_err(|error| error.to_string())? {
        Some(adapter) => adapter.local_root(&storage.provider_config),
        None => Ok(None),
    }
}

/// Presence of one declared secret in a draft.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StorageDraftSecretState {
    pub secret_id: SecretKey,
    pub present: bool,
    /// The machine-local draft value, carried only by the exact fetch of a
    /// draft that creates a new storage, so the user can review what the
    /// setup prepared. Listings, updates, and edit drafts of a
    /// committed storage never carry it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A storage setup draft as it crosses IPC: safe settings and secret
/// presence. Only `get_storage_setup_draft` on a new draft adds the values.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StorageSetupDraftSummary {
    pub draft_id: StorageSetupDraftId,
    pub storage_id: StorageId,
    pub name: String,
    pub provider: StorageProviderKey,
    pub config_version: u32,
    pub provider_label: String,
    pub available: bool,
    pub settings: BoundedConfigObject,
    pub secrets: Vec<StorageDraftSecretState>,
    /// The draft edits a storage that already exists in the configuration.
    pub edits_existing: bool,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

pub fn draft_summary(
    registry: &StorageProviderRegistry,
    draft: &StorageSetupDraft,
) -> StorageSetupDraftSummary {
    let storage = &draft.storage;
    let adapter = registry.get(&storage.provider, storage.provider_config.version);
    let descriptor = adapter.map(StorageProviderAdapter::descriptor);
    let secrets = secret_presence(descriptor.as_ref(), &storage.provider_config)
        .into_iter()
        .map(|presence| StorageDraftSecretState {
            secret_id: presence.secret_id,
            present: presence.present,
            value: None,
        })
        .collect();
    StorageSetupDraftSummary {
        draft_id: draft.draft_id.clone(),
        storage_id: storage.id.clone(),
        name: storage.name.clone(),
        provider: storage.provider.clone(),
        config_version: storage.provider_config.version,
        provider_label: descriptor
            .as_ref()
            .map(|descriptor| descriptor.display_name.clone())
            .unwrap_or_else(|| storage.provider.as_str().to_string()),
        available: adapter.is_some(),
        settings: storage.provider_config.settings.clone(),
        secrets,
        edits_existing: matches!(draft.source, StorageSetupDraftSource::Edit { .. }),
        revision: draft.revision,
        created_at: draft.created_at,
        updated_at: draft.updated_at,
    }
}

pub fn list_storage_setup_drafts(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
) -> Result<(Vec<StorageSetupDraftSummary>, Vec<String>), StorageSetupError> {
    let (config, drafts, warnings) = repository
        .list_storage_setup_drafts_with_config()
        .map_err(|_| StorageSetupError::persistence())?;
    Ok((
        drafts
            .iter()
            .filter(|draft| draft.validate_source_target(&config).is_ok())
            .map(|draft| draft_summary(registry, draft))
            .collect(),
        warnings,
    ))
}

/// The exact draft, opened by id. Unlike listings and update responses, a
/// draft that creates a new storage comes back with its secret values: they
/// are the assistant's or the user's own, and the form shows them for review
/// before Finish setup. An edit draft copies a committed secret and stays
/// redacted.
pub fn get_storage_setup_draft(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    draft_id: &StorageSetupDraftId,
) -> Result<Option<StorageSetupDraftSummary>, StorageSetupError> {
    let (config, draft) = repository
        .load_storage_setup_draft_with_config(draft_id)
        .map_err(|_| StorageSetupError::persistence())?;
    Ok(draft
        .filter(|draft| draft.validate_source_target(&config).is_ok())
        .map(|draft| {
            let mut summary = draft_summary(registry, &draft);
            if matches!(draft.source, StorageSetupDraftSource::New) {
                // Only declared form secrets can be reviewed. Listings and
                // edit drafts stay redacted, including OAuth credentials.
                let reviewable = registry
                    .get(
                        &draft.storage.provider,
                        draft.storage.provider_config.version,
                    )
                    .map(|adapter| {
                        adapter
                            .descriptor()
                            .secrets
                            .into_iter()
                            .map(|secret| secret.secret_id)
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                for state in &mut summary.secrets {
                    if !reviewable.contains(&state.secret_id) {
                        continue;
                    }
                    state.value = draft
                        .storage
                        .provider_config
                        .secrets
                        .get(&state.secret_id)
                        .map(|value| value.expose().to_string());
                }
            }
            summary
        }))
}

fn mint_draft_id() -> Result<StorageSetupDraftId, StorageSetupError> {
    StorageSetupDraftId::parse(
        generated_named_id("storagedraft").map_err(|_| StorageSetupError::persistence())?,
    )
    .map_err(|_| StorageSetupError::persistence())
}

fn mint_storage_id() -> Result<StorageId, StorageSetupError> {
    StorageId::parse(generated_named_id("storage").map_err(|_| StorageSetupError::persistence())?)
        .map_err(|_| StorageSetupError::persistence())
}

fn save_draft(
    repository: &LocalStateRepository,
    draft: StorageSetupDraft,
) -> Result<StorageSetupDraft, StorageSetupError> {
    repository
        .save_storage_setup_draft(draft)
        .map_err(map_storage_setup_draft_save_error)
}

/// Start a new draft for one compiled provider at its newest config version.
pub fn create_storage_setup_draft(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    provider: &StorageProviderKey,
) -> Result<StorageSetupDraftSummary, StorageSetupError> {
    let adapter = registry.latest(provider).ok_or_else(|| {
        StorageSetupError::invalid(
            "storage_provider_unknown",
            "This build has no storage provider with that key.",
        )
    })?;
    let descriptor = adapter.descriptor();
    let timestamp = unix_timestamp();
    let (drafts, _) = repository
        .list_storage_setup_drafts()
        .map_err(|_| StorageSetupError::persistence())?;
    // The draft opens under a placeholder numbered among this provider's
    // open drafts. The real name follows the destination once it is known,
    // and the commit makes it unique.
    let open = drafts
        .iter()
        .filter(|draft| draft.storage.provider == descriptor.key)
        .count();
    let name = placeholder_name(&descriptor.default_name, open);
    let saved = save_draft(
        repository,
        StorageSetupDraft {
            schema: STORAGE_SETUP_DRAFT_SCHEMA,
            draft_id: mint_draft_id()?,
            source: StorageSetupDraftSource::New,
            storage: StorageConfig {
                id: mint_storage_id()?,
                name,
                provider: descriptor.key,
                provider_config: StorageProviderConfig::new(
                    descriptor.config_version,
                    BoundedConfigObject::empty(),
                    SecretMap::empty(),
                ),
            },
            revision: 0,
            created_at: timestamp,
            updated_at: timestamp,
        },
    )?;
    Ok(draft_summary(registry, &saved))
}

/// Start a draft that edits an existing storage. The draft copies the saved
/// configuration, secrets included, so an unchanged secret survives commit
/// without ever being shown.
pub fn create_storage_edit_draft(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    storage_id: &StorageId,
) -> Result<StorageSetupDraftSummary, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    let storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(storage_not_found)?;
    registry
        .require(&storage.provider, storage.provider_config.version)
        .map_err(|_| storage_unavailable())?;
    let timestamp = unix_timestamp();
    let base_digest = storage
        .integrity_digest()
        .map_err(|_| StorageSetupError::persistence())?;
    let saved = save_draft(
        repository,
        StorageSetupDraft {
            schema: STORAGE_SETUP_DRAFT_SCHEMA,
            draft_id: mint_draft_id()?,
            source: StorageSetupDraftSource::Edit {
                storage_id: storage.id.clone(),
                base_digest,
            },
            storage: storage.clone(),
            revision: 0,
            created_at: timestamp,
            updated_at: timestamp,
        },
    )?;
    Ok(draft_summary(registry, &saved))
}

/// One focused draft edit: the display name, the safe settings, and a
/// presence operation per secret. Absent secrets are unchanged.
pub struct UpdateStorageSetupDraftRequest {
    pub draft_id: StorageSetupDraftId,
    pub revision: u64,
    pub name: String,
    pub settings: BoundedConfigObject,
    pub secrets: BTreeMap<SecretKey, SecretOperation>,
}

pub fn update_storage_setup_draft(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    request: UpdateStorageSetupDraftRequest,
) -> Result<StorageSetupDraftSummary, StorageSetupError> {
    let mut draft = load_storage_setup_draft(repository, &request.draft_id)?;
    if draft.revision != request.revision {
        return Err(storage_setup_draft_revision_conflict());
    }
    let adapter = registry
        .require(
            &draft.storage.provider,
            draft.storage.provider_config.version,
        )
        .map_err(|_| storage_unavailable())?;
    let descriptor = adapter.descriptor();
    let managed_id = descriptor
        .managed_secret_id
        .as_ref();
    let managed = managed_id.is_some_and(|key| draft.storage.provider_config.secrets.contains(key));
    let declared = descriptor
        .secrets
        .iter()
        .filter(|secret| {
            if managed {
                Some(&secret.secret_id) == managed_id
            } else {
                Some(&secret.secret_id) != managed_id
            }
        })
        .map(|secret| secret.secret_id.clone())
        .collect::<BTreeSet<_>>();
    // A saved storage's destination is fixed: an edit draft exists only to
    // replace a credential. The name has its own write (`rename_storage`).
    if matches!(draft.source, StorageSetupDraftSource::Edit { .. })
        && (request.settings != draft.storage.provider_config.settings
            || request.name != draft.storage.name)
    {
        return Err(storage_settings_locked());
    }
    if managed && request.secrets.keys().any(|key| !declared.contains(key)) {
        return Err(StorageSetupError::invalid(
            "storage_managed_credentials_locked",
            "This connection uses a refresh token. Replace that token or reconnect in Dropbox setup.",
        ));
    }
    for (key, operation) in request.secrets {
        if !declared.contains(&key) {
            return Err(StorageSetupError::invalid(
                "storage_secret_unknown",
                "The storage provider declares no secret with that identifier.",
            ));
        }
        draft
            .storage
            .provider_config
            .secrets
            .apply(&key, operation)
            .map_err(|_| {
                StorageSetupError::conflict(
                    "storage_secret_state_conflict",
                    "The secret changed since the form was loaded; reload the draft.",
                )
            })?;
    }
    draft.storage.name = request.name;
    draft.storage.provider_config.settings = request.settings;
    draft.updated_at = unix_timestamp().max(draft.updated_at);
    let saved = save_draft(repository, draft)?;
    Ok(draft_summary(registry, &saved))
}

pub fn discard_storage_setup_draft(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
) -> Result<bool, StorageSetupError> {
    repository
        .delete_storage_setup_draft(draft_id)
        .map_err(|_| StorageSetupError::persistence())
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TestStorageDraftResult {
    pub draft_id: StorageSetupDraftId,
    pub draft_revision: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CommitStorageDraftResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub replaced: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct VerifyStorageResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub verified: bool,
}

/// A draft pinned for testing or committing: the exact revision the user
/// reviewed, its adapter, and the exclusive host operation.
struct PinnedDraft<'a> {
    _operation: HostOperationGuard,
    draft: StorageSetupDraft,
    adapter: &'a dyn StorageProviderAdapter,
}

fn pin_draft<'a>(
    repository: &LocalStateRepository,
    registry: &'a StorageProviderRegistry,
    draft_id: &StorageSetupDraftId,
    expected_revision: u64,
) -> Result<PinnedDraft<'a>, StorageSetupError> {
    let operation = acquire_setup_operation(repository)?;
    let (config, draft) = load_storage_setup_draft_context(repository, draft_id)?;
    draft
        .validate_source(&config)
        .map_err(|_| storage_setup_draft_target_conflict())?;
    if draft.revision != expected_revision {
        return Err(storage_setup_draft_revision_conflict());
    }
    validate_name(&draft.storage.name)?;
    let adapter = registry
        .require(
            &draft.storage.provider,
            draft.storage.provider_config.version,
        )
        .map_err(|_| storage_unavailable())?;
    adapter
        .validate_config(&draft.storage.provider_config)
        .map_err(StorageSetupError::from_adapter)?;
    validate_settings_locked(&config, &draft.storage)?;
    ensure_destination_available(registry, &config, &draft.storage)?;
    Ok(PinnedDraft {
        _operation: operation,
        draft,
        adapter,
    })
}

/// Verify a drafted storage against its provider without committing
/// anything: the exact commit pipeline minus the config transaction. A test
/// never discards the draft.
pub fn test_storage_setup_draft(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    runtime: &Handle,
    draft_id: &StorageSetupDraftId,
    expected_revision: u64,
) -> Result<TestStorageDraftResult, StorageSetupError> {
    let pinned = pin_draft(repository, registry, draft_id, expected_revision)?;
    probe(pinned.adapter, runtime, &pinned.draft.storage)?;
    Ok(TestStorageDraftResult {
        draft_id: pinned.draft.draft_id.clone(),
        draft_revision: pinned.draft.revision,
        verified: true,
    })
}

/// Verify and atomically create or replace one storage from its draft. The
/// host operation stays held across the probe and the
/// storage-snapshot-guarded config transaction. `pre_commit` runs inside the
/// transaction on the prospective document so the caller can add checks the
/// application layer cannot see (local isolation, catalog links).
pub fn commit_storage_setup_draft(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    runtime: &Handle,
    draft_id: &StorageSetupDraftId,
    expected_draft_revision: u64,
    pre_commit: impl FnOnce(&SyncConfig) -> Result<(), String>,
) -> Result<CommitStorageDraftResult, StorageSetupError> {
    let pinned = pin_draft(repository, registry, draft_id, expected_draft_revision)?;
    probe(pinned.adapter, runtime, &pinned.draft.storage)?;

    let committed_draft = pinned.draft.clone();
    let mut storage = committed_draft.storage.clone();
    let source = committed_draft.source.clone();
    let stem = pinned.adapter.descriptor().default_name;
    let (replaced, committed) = repository
        .transact_config(move |config| {
            committed_draft
                .validate_source(config)
                .map_err(|_| storage_setup_draft_target_conflict())?;
            validate_settings_locked(config, &storage)?;
            ensure_destination_available(registry, config, &storage)?;
            // A committed name is unique: a placeholder falls back to the
            // stem, and a duplicate takes the next free number.
            let base = if is_placeholder_name(&storage.name, &stem) {
                stem.clone()
            } else {
                storage.name.clone()
            };
            let id = storage.id.clone();
            storage.name = unique_storage_name(&base, |candidate| {
                config
                    .storages
                    .iter()
                    .any(|other| other.id != id && other.name == candidate)
            });
            let replaced = match &source {
                StorageSetupDraftSource::New => {
                    config.storages.push(storage);
                    false
                }
                StorageSetupDraftSource::Edit { storage_id, .. } => {
                    let index = config
                        .storages
                        .iter()
                        .position(|existing| &existing.id == storage_id)
                        .ok_or_else(storage_setup_draft_target_conflict)?;
                    config.storages[index] = storage;
                    true
                }
            };
            pre_commit(config).map_err(|message| {
                StorageSetupError::invalid_text("storage_commit_rejected", message)
            })?;
            Ok(ConfigMutation::Changed(replaced))
        })
        .map_err(|error| match error {
            ConfigTransactionError::Mutation(error) => error,
            ConfigTransactionError::Persistence(_) => StorageSetupError::persistence(),
        })?;
    // The draft served its purpose the moment the commit landed. Discard is
    // best-effort and never fails a committed storage.
    let _ = repository.delete_storage_setup_draft(draft_id);
    Ok(CommitStorageDraftResult {
        storage_id: pinned.draft.storage.id.clone(),
        config_revision: committed.revision,
        replaced,
    })
}

impl StorageSetupError {
    fn invalid_text(code: &'static str, message: String) -> Self {
        Self {
            kind: StorageSetupErrorKind::InvalidInput,
            code,
            message,
        }
    }

    /// A conflict whose sentence carries a storage's display name: the one
    /// value in the error that the user typed and can act on. Never a
    /// setting, secret, or destination.
    fn conflict_text(code: &'static str, message: String) -> Self {
        Self {
            kind: StorageSetupErrorKind::Conflict,
            code,
            message,
        }
    }
}

/// Rename one storage under a config revision check.
pub fn rename_storage(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    name: &str,
    expected_config_revision: u64,
) -> Result<u64, StorageSetupError> {
    validate_name(name)?;
    let storage_id = storage_id.clone();
    let name = name.to_string();
    let (_, committed) = repository
        .transact_config(move |config| {
            if config.revision != expected_config_revision {
                return Err(revision_conflict());
            }
            // A duplicate takes the next free number, as at commit.
            let name = unique_storage_name(&name, |candidate| {
                config
                    .storages
                    .iter()
                    .any(|other| other.id != storage_id && other.name == candidate)
            });
            let storage = config
                .storages
                .iter_mut()
                .find(|storage| storage.id == storage_id)
                .ok_or_else(storage_not_found)?;
            if storage.name == name {
                return Ok(ConfigMutation::Unchanged(()));
            }
            storage.name = name;
            Ok(ConfigMutation::Changed(()))
        })
        .map_err(|error| match error {
            ConfigTransactionError::Mutation(error) => error,
            ConfigTransactionError::Persistence(_) => StorageSetupError::persistence(),
        })?;
    Ok(committed.revision)
}

/// Remove one storage record. Stored files are never touched. A storage that
/// a project still links is refused; `pre_remove` lets the caller refuse for
/// reasons the application layer cannot see.
pub fn remove_storage(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    expected_config_revision: u64,
    pre_remove: impl FnOnce(&StorageConfig) -> Result<(), String>,
) -> Result<u64, StorageSetupError> {
    let _operation = acquire_setup_operation(repository)?;
    let snapshot = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    if snapshot.revision != expected_config_revision {
        return Err(revision_conflict());
    }
    let stored = snapshot
        .storages
        .iter()
        .find(|s| &s.id == storage_id)
        .ok_or_else(storage_not_found)?;
    // Cross-document checks run under the host guard, outside either document lock.
    pre_remove(stored)
        .map_err(|message| StorageSetupError::invalid_text("storage_removal_rejected", message))?;
    let removed_connection = (stored.provider
        == mallard_infrastructure::storage_providers::feishu::provider_key())
    .then(|| stored.provider_config.clone());
    let removed_id = storage_id.clone();
    let storage_id = storage_id.clone();
    let (_, committed) = repository
        .transact_config(move |config| {
            if config.revision != expected_config_revision {
                return Err(revision_conflict());
            }
            let index = config
                .storages
                .iter()
                .position(|storage| storage.id == storage_id)
                .ok_or_else(storage_not_found)?;
            if config.links.iter().any(|link| link.storage_id == storage_id) {
                return Err(StorageSetupError::conflict(
                    "storage_linked",
                    "This storage is still linked to a project. Unlink it before removing the storage.",
                ));
            }
            config.storages.remove(index);
            Ok(ConfigMutation::Changed(()))
        })
        .map_err(|error| match error {
            ConfigTransactionError::Mutation(error) => error,
            ConfigTransactionError::Persistence(_) => StorageSetupError::persistence(),
        })?;
    ConfiguredObjectStore::forget(&removed_id);
    // The sign-in this computer kept for the storage goes with it; the
    // folder in Feishu is untouched.
    if let Some(config) = removed_connection {
        mallard_infrastructure::storage_providers::feishu::forget_connection(&config);
    }
    Ok(committed.revision)
}

/// Verify one saved storage against its provider without mutating anything.
pub fn verify_storage(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    runtime: &Handle,
    storage_id: &StorageId,
) -> Result<VerifyStorageResult, StorageSetupError> {
    let _operation = acquire_setup_operation(repository)?;
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    let storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(storage_not_found)?;
    let adapter = registry
        .require(&storage.provider, storage.provider_config.version)
        .map_err(|_| storage_unavailable())?;
    adapter
        .validate_config(&storage.provider_config)
        .map_err(StorageSetupError::from_adapter)?;
    probe(adapter, runtime, storage)?;
    Ok(VerifyStorageResult {
        storage_id: storage.id.clone(),
        config_revision: config.revision,
        verified: true,
    })
}

/// The descriptors of every compiled provider, in display order.
pub fn storage_provider_descriptors(
    registry: &StorageProviderRegistry,
) -> Vec<StorageProviderDescriptor> {
    registry.descriptors()
}

/// Render the declared setup assistant's instruction for one exact draft.
/// A provider that declares no assistant is refused.


/// Exclusive host-operation token for preparing one existing setup draft
/// from direct setup. The caller keeps this value alive across its
/// provider-specific work. The draft revision is pinned before that work so
/// a concurrent form edit cannot be overwritten when setup finishes.
pub struct StorageDraftOperation {
    repository_root: PathBuf,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    name: String,
    provider: StorageProviderKey,
    config_version: u32,
    _guard: HostOperationGuard,
}

impl fmt::Debug for StorageDraftOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageDraftOperation")
            .field("draft_id", &self.draft_id)
            .field("expected_revision", &self.expected_revision)
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl StorageDraftOperation {
    pub(crate) fn draft_id(&self) -> &StorageSetupDraftId {
        &self.draft_id
    }

    pub fn provider(&self) -> &StorageProviderKey {
        &self.provider
    }

    pub fn config_version(&self) -> u32 {
        self.config_version
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Validate and pin the exact editable draft before setup does any
/// provider-specific or browser work.
pub fn begin_storage_draft_operation(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
    expected_revision: u64,
    name: &str,
) -> Result<StorageDraftOperation, StorageSetupError> {
    validate_name(name)?;
    let guard = acquire_setup_operation(repository)?;
    let draft = load_storage_setup_draft_for_operation(repository, draft_id)?;
    validate_storage_setup_draft_target(&draft, expected_revision, name)?;
    Ok(StorageDraftOperation {
        repository_root: repository.root().to_path_buf(),
        draft_id: draft_id.clone(),
        expected_revision,
        name: name.to_string(),
        provider: draft.storage.provider.clone(),
        config_version: draft.storage.provider_config.version,
        _guard: guard,
    })
}

/// Prepared settings and secrets for the pinned draft's provider.
/// Deliberately has no `Debug`, `Clone`, or `Serialize`.
pub struct PreparedProviderConfig {
    pub settings: BoundedConfigObject,
    pub secrets: SecretMap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepareStorageDraftResult {
    pub draft_id: StorageSetupDraftId,
    pub draft_revision: u64,
}

/// Put prepared values into the pinned draft without probing or
/// mutating the committed configuration. The user reviews, optionally
/// tests, and explicitly commits in Mallard.
pub fn prepare_storage_draft_in_operation(
    operation: &StorageDraftOperation,
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    prepared: PreparedProviderConfig,
) -> Result<PrepareStorageDraftResult, StorageSetupError> {
    if operation.repository_root != repository.root() {
        return Err(StorageSetupError::persistence());
    }
    let adapter = registry
        .require(&operation.provider, operation.config_version)
        .map_err(|_| storage_unavailable())?;
    let settings = adapter
        .validate_settings(&prepared.settings)
        .map_err(StorageSetupError::from_adapter)?;
    let provider_config =
        StorageProviderConfig::new(operation.config_version, settings, prepared.secrets);
    adapter
        .validate_config(&provider_config)
        .map_err(StorageSetupError::from_adapter)?;

    let mut draft = load_storage_setup_draft_for_operation(repository, &operation.draft_id)?;
    validate_storage_setup_draft_target(&draft, operation.expected_revision, &operation.name)?;
    if draft.storage.provider != operation.provider {
        return Err(StorageSetupError::conflict(
            "storage_setup_draft_provider_conflict",
            "The storage setup draft belongs to a different provider.",
        ));
    }
    draft.storage.name = operation.name.clone();
    draft.storage.provider_config = provider_config;
    draft.updated_at = unix_timestamp().max(draft.updated_at);
    let saved = save_draft(repository, draft)?;
    Ok(PrepareStorageDraftResult {
        draft_id: saved.draft_id.clone(),
        draft_revision: saved.revision,
    })
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn acquire_setup_operation(
    repository: &LocalStateRepository,
) -> Result<HostOperationGuard, StorageSetupError> {
    repository.acquire_host_operation().map_err(|error| {
        if error == "another host-sensitive operation is already in progress" {
            StorageSetupError::conflict(
                "storage_setup_busy",
                "Another Mallard host operation is already in progress.",
            )
        } else {
            StorageSetupError::persistence()
        }
    })
}

fn load_storage_setup_draft(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
) -> Result<StorageSetupDraft, StorageSetupError> {
    load_storage_setup_draft_context(repository, draft_id).map(|(_, draft)| draft)
}

fn load_storage_setup_draft_context(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
) -> Result<(SyncConfig, StorageSetupDraft), StorageSetupError> {
    let (config, draft) = repository
        .load_storage_setup_draft_with_config(draft_id)
        .map_err(|_| StorageSetupError::persistence())?;
    let draft = draft.ok_or_else(storage_setup_draft_not_found)?;
    draft
        .validate_source_target(&config)
        .map_err(|_| storage_setup_draft_target_conflict())?;
    Ok((config, draft))
}

fn load_storage_setup_draft_for_operation(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
) -> Result<StorageSetupDraft, StorageSetupError> {
    let (config, draft) = load_storage_setup_draft_context(repository, draft_id)?;
    draft
        .validate_source(&config)
        .map_err(|_| storage_setup_draft_target_conflict())?;
    Ok(draft)
}

fn validate_storage_setup_draft_target(
    draft: &StorageSetupDraft,
    expected_revision: u64,
    name: &str,
) -> Result<(), StorageSetupError> {
    if draft.revision != expected_revision {
        return Err(storage_setup_draft_revision_conflict());
    }
    // The editable form may persist surrounding whitespace before its copied
    // instructions normalize the display name. `name` itself is already
    // strict and trimmed, so compare the same normalized form and write that
    // canonical value when preparation finishes.
    if draft.storage.name.trim() != name {
        return Err(StorageSetupError::conflict(
            "storage_setup_draft_name_conflict",
            "The storage setup draft name changed; copy fresh agent setup instructions from Mallard.",
        ));
    }
    Ok(())
}

fn map_storage_setup_draft_save_error(error: String) -> StorageSetupError {
    if error.starts_with("storage draft changed") {
        storage_setup_draft_revision_conflict()
    } else if error.starts_with("storage draft target changed")
        || error.starts_with("storage draft source changed")
    {
        storage_setup_draft_target_conflict()
    } else if error.starts_with("too many storage drafts") {
        StorageSetupError::conflict(
            "storage_setup_draft_limit",
            "Too many storage drafts are open; discard one first.",
        )
    } else if error.contains("draft storage name") {
        StorageSetupError::invalid("storage_name_invalid", "The storage name is invalid.")
    } else {
        StorageSetupError::persistence()
    }
}

/// The continuity identity of one storage under this registry: provider key
/// plus canonical coordinates. This operating path requires a registered,
/// fully valid provider configuration and resolvable coordinates.
fn continuity_identity(
    registry: &StorageProviderRegistry,
    storage: &StorageConfig,
) -> Result<(StorageProviderKey, Vec<String>), StorageSetupError> {
    let adapter =
        validated_registered_adapter(registry, storage)?.ok_or_else(storage_unavailable)?;
    let coordinates = adapter
        .continuity_coordinates(&storage.provider_config)
        .map_err(|_| {
            StorageSetupError::invalid(
                "storage_destination_invalid",
                "The storage destination could not be resolved.",
            )
        })?;
    Ok((storage.provider.clone(), coordinates))
}

/// A destination may be connected once regardless of credentials or display
/// name. The candidate's own id is excluded so an edit can keep its place.
/// The refusal names the storage that holds the destination, so the page
/// can send the user to it instead of leaving them at a dead field.
fn ensure_destination_available(
    registry: &StorageProviderRegistry,
    config: &SyncConfig,
    candidate: &StorageConfig,
) -> Result<(), StorageSetupError> {
    let identity = continuity_identity(registry, candidate)?;
    for storage in config
        .storages
        .iter()
        .filter(|storage| storage.id != candidate.id && storage.provider == candidate.provider)
    {
        if continuity_identity(registry, storage)? == identity {
            return Err(StorageSetupError::conflict_text(
                "storage_destination_conflict",
                format!(
                    "That storage destination is already connected as \u{201c}{}\u{201d}.",
                    storage.name
                ),
            ));
        }
    }
    Ok(())
}

/// Verify the candidate configuration remotely. Nothing is recorded
/// locally: an adapter that writes names its probe object itself and
/// removes it best effort.
fn probe(
    adapter: &dyn StorageProviderAdapter,
    runtime: &Handle,
    pending: &StorageConfig,
) -> Result<(), StorageSetupError> {
    adapter
        .probe(&pending.provider_config, runtime)
        .map_err(StorageSetupError::from_adapter)?;
    // The probe proved the connection with a fresh transport; a shared
    // store that recorded a rejection for this storage starts over.
    ConfiguredObjectStore::forget(&pending.id);
    Ok(())
}

/// The name a draft opens with, numbered among the provider's open drafts.
fn placeholder_name(stem: &str, open_drafts: usize) -> String {
    format!("{stem} draft {}", open_drafts + 1)
}

/// Whether `name` is still the placeholder a draft opened with.
fn is_placeholder_name(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix(" draft "))
        .is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// `name`, or the first of `name 2`, `name 3`, … that is not `taken`.
fn unique_storage_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    (2u32..)
        .map(|number| format!("{name} {number}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_string())
}

fn validate_name(name: &str) -> Result<(), StorageSetupError> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > MAX_STORAGE_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        Err(StorageSetupError::invalid(
            "storage_name_invalid",
            "The storage name is invalid.",
        ))
    } else {
        Ok(())
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().max(1))
        .unwrap_or(1)
}

fn storage_setup_draft_not_found() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_setup_draft_not_found",
        "The requested storage setup draft does not exist.",
    )
}

fn storage_setup_draft_revision_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "storage_setup_draft_revision_conflict",
        "The storage setup draft changed; review the latest settings in Mallard.",
    )
}

fn storage_setup_draft_target_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "storage_setup_draft_target_conflict",
        "The storage changed or was removed after this draft opened; discard the draft and try again.",
    )
}

fn revision_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "config_revision_conflict",
        "Mallard storage configuration changed; reload it before saving storage.",
    )
}

fn storage_not_found() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_not_found",
        "The requested Mallard storage does not exist.",
    )
}

fn storage_unavailable() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_provider_unavailable",
        "This build has no storage provider for that storage's provider and configuration version.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::LocalProjectId;
    use mallard_infrastructure::storage_providers::local::{
        local_storage_config,
    };
    use mallard_infrastructure::storage_providers::test_provider::{
        provider_key as test_key, test_storage_config,
    };

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap()
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn test_settings(directory: &std::path::Path) -> BoundedConfigObject {
        BoundedConfigObject::from_json(&serde_json::json!({
            "directory": directory.to_string_lossy(),
            "label": "proof",
        }))
        .unwrap()
    }

    fn token() -> SecretKey {
        SecretKey::parse("token").unwrap()
    }

    #[test]
    fn the_exact_fetch_of_a_new_draft_reveals_its_own_secret_and_an_edit_draft_stays_redacted() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let draft = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        let store_dir = temp.path().join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        let updated = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: draft.revision,
                name: "Revealed".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::from([(
                    token(),
                    SecretOperation::Set {
                        value: mallard_domain::SecretValue::parse("minted-by-the-assistant")
                            .unwrap(),
                    },
                )]),
            },
        )
        .unwrap();
        assert!(
            updated.secrets.iter().all(|state| state.value.is_none()),
            "update responses report presence only"
        );
        let listed = list_storage_setup_drafts(&repository, &registry).unwrap().0;
        assert!(listed
            .iter()
            .all(|listed| listed.secrets.iter().all(|state| state.value.is_none())));

        let exact = get_storage_setup_draft(&repository, &registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            exact.secrets,
            vec![StorageDraftSecretState {
                secret_id: token(),
                present: true,
                value: Some("minted-by-the-assistant".to_string()),
            }],
            "the exact fetch of a new draft carries its own value for review"
        );

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let committed = commit_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &draft.draft_id,
            updated.revision,
            |_| Ok(()),
        )
        .unwrap();
        let edit =
            create_storage_edit_draft(&repository, &registry, &committed.storage_id).unwrap();
        let exact_edit = get_storage_setup_draft(&repository, &registry, &edit.draft_id)
            .unwrap()
            .unwrap();
        assert!(exact_edit.edits_existing);
        assert!(
            exact_edit.secrets.iter().any(|state| state.present)
                && exact_edit.secrets.iter().all(|state| state.value.is_none()),
            "a committed secret copied into an edit draft is never revealed"
        );
    }

    #[test]
    fn drafts_report_secret_presence_only_and_pin_revisions() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let draft = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        assert_eq!(draft.revision, 1);
        assert_eq!(draft.config_version, 1);
        assert!(draft.available);
        assert!(!draft.edits_existing);
        assert_eq!(
            draft.secrets,
            vec![StorageDraftSecretState {
                secret_id: token(),
                present: false,
                value: None
            }]
        );

        let store_dir = temp.path().join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        let updated = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: 1,
                name: "Proof".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::from([(
                    token(),
                    SecretOperation::Set {
                        value: SecretValue::parse("token-canary").unwrap(),
                    },
                )]),
            },
        )
        .unwrap();
        assert_eq!(updated.revision, 2);
        assert!(updated.secrets[0].present);
        let rendered = serde_json::to_string(&updated).unwrap();
        assert!(!rendered.contains("token-canary"));
        let listed = list_storage_setup_drafts(&repository, &registry).unwrap().0;
        assert_eq!(listed, vec![updated.clone()]);
        // The exact fetch of a new draft is the one view that carries the
        // value, for review; everything else about it matches the update.
        let mut exact = get_storage_setup_draft(&repository, &registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(exact.secrets[0].value.as_deref(), Some("token-canary"));
        exact.secrets[0].value = None;
        assert_eq!(exact, updated);

        // Stale revision, unknown secret, and a Set over a present secret
        // are all refused without changing the draft.
        let stale = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: 1,
                name: "Proof".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap_err();
        assert_eq!(stale.code(), "storage_setup_draft_revision_conflict");
        let unknown = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: 2,
                name: "Proof".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::from([(
                    SecretKey::parse("other").unwrap(),
                    SecretOperation::Clear,
                )]),
            },
        )
        .unwrap_err();
        assert_eq!(unknown.code(), "storage_secret_unknown");
        let reset = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: 2,
                name: "Proof".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::from([(
                    token(),
                    SecretOperation::Set {
                        value: SecretValue::parse("second").unwrap(),
                    },
                )]),
            },
        )
        .unwrap_err();
        assert_eq!(reset.code(), "storage_secret_state_conflict");
        let cleared = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: 2,
                name: "Proof".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::from([(token(), SecretOperation::Clear)]),
            },
        )
        .unwrap();
        assert!(!cleared.secrets[0].present);
    }

    #[test]
    fn unknown_provider_cannot_start_a_draft_but_its_record_still_lists() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::production();
        let error = create_storage_setup_draft(
            &repository,
            registry,
            &StorageProviderKey::parse("unregistered").unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "storage_provider_unknown");

        // A record written by a build with more providers stays listable and
        // inert: shown as unavailable, never edited or tested.
        let foreign = StorageConfig {
            id: StorageId::parse("storage-foreign").unwrap(),
            name: "Foreign".to_string(),
            provider: StorageProviderKey::parse("unregistered").unwrap(),
            provider_config: StorageProviderConfig::new(
                1,
                BoundedConfigObject::from_json(&serde_json::json!({"namespace_id": "ns"})).unwrap(),
                SecretMap::empty(),
            ),
        };
        repository
            .mutate_config(|config| {
                config.storages.push(foreign.clone());
                Ok(())
            })
            .unwrap();
        let listed = list_storages(&repository, registry).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].available);
        assert_eq!(listed[0].provider_label, "unregistered");
        assert!(listed[0].summary.is_empty());
        assert_eq!(
            create_storage_edit_draft(&repository, registry, &foreign.id)
                .unwrap_err()
                .code(),
            "storage_provider_unavailable"
        );
        let stored = repository.load_config().unwrap();
        assert_eq!(stored.storages, vec![foreign]);
    }

    #[test]
    fn registered_invalid_configs_fail_closed_before_listing_roots_and_uniqueness() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::production();
        let mut invalid = local_storage_config(
            StorageId::parse("storage-invalid").unwrap(),
            "Invalid",
            &temp.path().join("invalid-store"),
        );
        invalid
            .provider_config
            .secrets
            .insert(
                SecretKey::parse("unexpected").unwrap(),
                SecretValue::parse("secret-canary").unwrap(),
            )
            .unwrap();
        assert!(invalid.validate().is_ok());
        repository
            .mutate_config(|config| {
                config.storages.push(invalid.clone());
                Ok(())
            })
            .unwrap();

        let error = list_storages(&repository, registry).unwrap_err();
        assert_eq!(error.code(), "local_secrets_not_allowed");
        assert!(!error.to_string().contains("secret-canary"));
        let root_error = storage_local_root(registry, &invalid).unwrap_err();
        assert_eq!(root_error, "A local folder storage has no secrets.");

        let candidate = local_storage_config(
            StorageId::parse("storage-candidate").unwrap(),
            "Candidate",
            &temp.path().join("candidate-store"),
        );
        let config = repository.load_config().unwrap();
        let error = ensure_destination_available(registry, &config, &candidate).unwrap_err();
        assert_eq!(error.code(), "local_secrets_not_allowed");

        let mut unsupported = local_storage_config(
            StorageId::parse("storage-unsupported").unwrap(),
            "Unsupported",
            &temp.path().join("unsupported-store"),
        );
        unsupported.provider_config.version = 2;
        let config = SyncConfig {
            storages: vec![unsupported],
            ..SyncConfig::default()
        };
        let error = ensure_destination_available(registry, &config, &candidate).unwrap_err();
        assert_eq!(error.code(), "storage_provider_unavailable");
    }

    #[test]
    fn a_new_draft_opens_under_a_placeholder_numbered_among_its_provider_drafts() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let first = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        assert_eq!(first.name, "Test storage draft 1");
        let second = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        assert_eq!(second.name, "Test storage draft 2");
        // Another provider counts its own drafts.
        let local = create_storage_setup_draft(
            &repository,
            &registry,
            &mallard_infrastructure::storage_providers::local::provider_key(),
        )
        .unwrap();
        assert_eq!(local.name, "Local storage draft 1");
        assert!(is_placeholder_name("Test storage draft 12", "Test storage"));
        for name in [
            "Test storage",
            "Test storage draft",
            "Test storage draft x",
            "myconfstorage",
        ] {
            assert!(!is_placeholder_name(name, "Test storage"), "{name}");
        }
    }

    #[test]
    fn a_committed_name_is_unique_and_a_placeholder_falls_back_to_the_stem() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let runtime = runtime();
        let commit = |name: Option<&str>, folder: &str| -> String {
            let store_dir = temp.path().join(folder);
            std::fs::create_dir_all(&store_dir).unwrap();
            let draft = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
            let draft = update_storage_setup_draft(
                &repository,
                &registry,
                UpdateStorageSetupDraftRequest {
                    draft_id: draft.draft_id.clone(),
                    revision: draft.revision,
                    name: name.map(str::to_string).unwrap_or(draft.name),
                    settings: test_settings(&store_dir),
                    secrets: BTreeMap::new(),
                },
            )
            .unwrap();
            let committed = commit_storage_setup_draft(
                &repository,
                &registry,
                runtime.handle(),
                &draft.draft_id,
                draft.revision,
                |_| Ok(()),
            )
            .unwrap();
            repository
                .load_config()
                .unwrap()
                .storages
                .into_iter()
                .find(|storage| storage.id == committed.storage_id)
                .unwrap()
                .name
        };
        assert_eq!(
            commit(Some("myconfstorage"), "a/myconfstorage"),
            "myconfstorage"
        );
        assert_eq!(
            commit(Some("myconfstorage"), "b/myconfstorage"),
            "myconfstorage 2"
        );
        assert_eq!(
            commit(Some("myconfstorage"), "c/myconfstorage"),
            "myconfstorage 3"
        );
        // A draft committed under its placeholder takes the stem, then a number.
        assert_eq!(commit(None, "d"), "Test storage");
        assert_eq!(commit(None, "e"), "Test storage 2");
    }

    #[test]
    fn rename_takes_the_next_free_number_for_a_duplicate() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let first = StorageId::parse("storage-first").unwrap();
        let second = StorageId::parse("storage-second").unwrap();
        repository
            .mutate_config(|config| {
                config.storages.push(local_storage_config(
                    first.clone(),
                    "Backups",
                    &temp.path().join("first"),
                ));
                config.storages.push(local_storage_config(
                    second.clone(),
                    "Other",
                    &temp.path().join("second"),
                ));
                Ok(())
            })
            .unwrap();
        let revision = repository.load_config().unwrap().revision;
        rename_storage(&repository, &second, "Backups", revision).unwrap();
        let names = || -> Vec<String> {
            repository
                .load_config()
                .unwrap()
                .storages
                .iter()
                .map(|storage| storage.name.clone())
                .collect()
        };
        assert_eq!(names(), ["Backups", "Backups 2"]);
        // A storage's own name is never a collision with itself.
        let revision = repository.load_config().unwrap().revision;
        rename_storage(&repository, &first, "Backups", revision).unwrap();
        assert_eq!(names(), ["Backups", "Backups 2"]);
    }

    #[test]
    fn test_and_commit_probe_and_commit_under_snapshot_checks() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let runtime = runtime();
        let store_dir = temp.path().join("store");
        let draft = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        let draft = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: draft.revision,
                name: "Proof".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap();

        // The folder does not exist yet: the adapter's probe fails remotely
        // and nothing is committed.
        let failed = test_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &draft.draft_id,
            draft.revision,
        )
        .unwrap_err();
        assert_eq!(failed.kind(), StorageSetupErrorKind::RemoteFailure);
        assert_eq!(failed.code(), "teststore_unreachable");
        assert!(repository.load_config().unwrap().storages.is_empty());

        std::fs::create_dir_all(&store_dir).unwrap();
        let verified = test_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &draft.draft_id,
            draft.revision,
        )
        .unwrap();
        assert!(verified.verified);
        assert!(repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .is_some());

        let rejected = commit_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &draft.draft_id,
            draft.revision,
            |_| Err("overlaps".to_string()),
        )
        .unwrap_err();
        assert_eq!(rejected.code(), "storage_commit_rejected");
        assert!(repository.load_config().unwrap().storages.is_empty());

        let committed = commit_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &draft.draft_id,
            draft.revision,
            |config| {
                assert_eq!(config.storages.len(), 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(committed.storage_id, draft.storage_id);
        assert!(!committed.replaced);
        let config = repository.load_config().unwrap();
        assert_eq!(config.revision, committed.config_revision);
        assert_eq!(config.storages[0].name, "Proof");
        assert!(repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .is_none());

        // The same destination cannot be connected twice.
        let second = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        let second = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: second.draft_id.clone(),
                revision: second.revision,
                name: "Again".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap();
        let duplicate = test_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &second.draft_id,
            second.revision,
        )
        .unwrap_err();
        assert_eq!(duplicate.code(), "storage_destination_conflict");

        // An edit draft keeps the storage id and replaces the record.
        let edit =
            create_storage_edit_draft(&repository, &registry, &committed.storage_id).unwrap();
        assert!(edit.edits_existing);
        assert_eq!(edit.storage_id, committed.storage_id);
        // The name and the settings are fixed on an edit draft.
        let renamed = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: edit.draft_id.clone(),
                revision: edit.revision,
                name: "Proof renamed".to_string(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap_err();
        assert_eq!(renamed.code(), "storage_settings_locked");
        let original_name = edit.name.clone();
        let edit = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: edit.draft_id.clone(),
                revision: edit.revision,
                name: original_name.clone(),
                settings: test_settings(&store_dir),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap();
        let replaced = commit_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &edit.draft_id,
            edit.revision,
            |_| Ok(()),
        )
        .unwrap();
        assert!(replaced.replaced);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages.len(), 1);
        assert_eq!(config.storages[0].name, original_name);
    }

    #[cfg(unix)]
    #[test]
    fn local_storage_parent_symlink_aliases_have_one_destination() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::production();
        let real = temp.path().join("real");
        let store = real.join("store");
        std::fs::create_dir_all(&store).unwrap();
        let alias = temp.path().join("alias");
        symlink(&real, &alias).unwrap();

        repository
            .mutate_config(|config| {
                config.storages.push(local_storage_config(
                    StorageId::parse("storage-real").unwrap(),
                    "Real",
                    &store,
                ));
                Ok(())
            })
            .unwrap();
        let config = repository.load_config().unwrap();
        let candidate = local_storage_config(
            StorageId::parse("storage-alias").unwrap(),
            "Alias",
            &alias.join("store"),
        );

        let error = ensure_destination_available(registry, &config, &candidate).unwrap_err();
        assert_eq!(error.code(), "storage_destination_conflict");
        // The sentence names the storage that owns the folder, nothing else
        // about it: no path, no id.
        assert_eq!(
            error.to_string(),
            "That storage destination is already connected as \u{201c}Real\u{201d}."
        );
        assert!(!error.to_string().contains("store"));
    }

    #[test]
    fn unresolvable_registered_destination_is_not_treated_as_unique() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let anchor = temp.path().join("not-a-directory");
        std::fs::write(&anchor, b"file").unwrap();
        let candidate = local_storage_config(
            StorageId::parse("storage-invalid").unwrap(),
            "Invalid",
            &anchor.join("store"),
        );

        let error = ensure_destination_available(
            StorageProviderRegistry::production(),
            &repository.load_config().unwrap(),
            &candidate,
        )
        .unwrap_err();
        assert_eq!(error.code(), "storage_destination_invalid");
    }

    #[test]
    fn edit_draft_commit_uses_its_storage_snapshot_not_the_global_config_revision() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let runtime = runtime();
        let storage_id = StorageId::parse("storage-edited").unwrap();
        let store_dir = temp.path().join("edited-store");
        let unrelated_dir = temp.path().join("unrelated-store");
        std::fs::create_dir_all(&store_dir).unwrap();
        std::fs::create_dir_all(&unrelated_dir).unwrap();
        repository
            .mutate_config(|config| {
                config.storages.push(test_storage_config(
                    storage_id.clone(),
                    "Original",
                    &store_dir,
                ));
                Ok(())
            })
            .unwrap();

        let edit = create_storage_edit_draft(&repository, &registry, &storage_id).unwrap();
        let edit = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: edit.draft_id,
                revision: edit.revision,
                name: "Original".to_string(),
                settings: edit.settings.clone(),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap();
        repository
            .mutate_config(|config| {
                config.storages.push(test_storage_config(
                    StorageId::parse("storage-unrelated").unwrap(),
                    "Unrelated",
                    &unrelated_dir,
                ));
                Ok(())
            })
            .unwrap();

        let committed = commit_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &edit.draft_id,
            edit.revision,
            |_| Ok(()),
        )
        .unwrap();

        assert!(committed.replaced);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages.len(), 2);
        assert_eq!(
            config
                .storages
                .iter()
                .find(|storage| storage.id == storage_id)
                .unwrap()
                .name,
            "Original"
        );

        let stale = create_storage_edit_draft(&repository, &registry, &storage_id).unwrap();
        repository
            .mutate_config(|config| {
                config
                    .storages
                    .iter_mut()
                    .find(|storage| storage.id == storage_id)
                    .unwrap()
                    .name = "Changed elsewhere".to_string();
                Ok(())
            })
            .unwrap();
        let error = commit_storage_setup_draft(
            &repository,
            &registry,
            runtime.handle(),
            &stale.draft_id,
            stale.revision,
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.code(), "storage_setup_draft_target_conflict");
    }

    #[test]
    fn removing_storage_invalidates_only_its_edit_drafts() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let storage_id = StorageId::parse("storage-removed").unwrap();
        let store_dir = temp.path().join("removed-store");
        repository
            .mutate_config(|config| {
                config.storages.push(test_storage_config(
                    storage_id.clone(),
                    "Removed",
                    &store_dir,
                ));
                Ok(())
            })
            .unwrap();
        let edit = create_storage_edit_draft(&repository, &registry, &storage_id).unwrap();
        let fresh = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();

        let revision = repository.load_config().unwrap().revision;
        remove_storage(&repository, &storage_id, revision, |_| Ok(())).unwrap();

        assert!(
            get_storage_setup_draft(&repository, &registry, &edit.draft_id)
                .unwrap()
                .is_none()
        );
        let listed = list_storage_setup_drafts(&repository, &registry).unwrap().0;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].draft_id, fresh.draft_id);
    }

    #[test]
    fn rename_and_remove_are_focused_and_revision_checked() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let storage_id = StorageId::parse("storage-local").unwrap();
        let store_dir = temp.path().join("store");
        repository
            .mutate_config(|config| {
                config.storages.push(local_storage_config(
                    storage_id.clone(),
                    "Local",
                    &store_dir,
                ));
                Ok(())
            })
            .unwrap();
        let revision = repository.load_config().unwrap().revision;
        assert_eq!(
            rename_storage(&repository, &storage_id, "Renamed", revision + 1)
                .unwrap_err()
                .code(),
            "config_revision_conflict"
        );
        assert_eq!(
            rename_storage(&repository, &storage_id, " padded", revision)
                .unwrap_err()
                .code(),
            "storage_name_invalid"
        );
        let renamed = rename_storage(&repository, &storage_id, "Renamed", revision).unwrap();
        assert_eq!(renamed, revision + 1);
        assert_eq!(
            repository.load_config().unwrap().storages[0].name,
            "Renamed"
        );

        let project_id = LocalProjectId::parse("project-a").unwrap();
        repository
            .mutate_config(|config| {
                config
                    .projects
                    .push(mallard_domain::LocalProjectRegistration {
                        local_project_id: project_id.clone(),
                        bundle_id: mallard_domain::BundleId::parse(
                            "0123456789abcdef0123456789abcdef",
                        )
                        .unwrap(),
                        display_name: "Project".to_string(),
                        local_alias: None,
                        repository_fingerprint: None,
                        recipe: mallard_domain::BundleRecipe::default(),
                        revision: 0,
                        created_at: 1,
                        updated_at: 1,
                    });
                config.links.push(mallard_domain::ProjectStorageLink {
                    managed_cloud_repository_id: None,
                    local_project_id: project_id.clone(),
                    storage_id: storage_id.clone(),
                    bundle_id: mallard_domain::BundleId::parse("0123456789abcdef0123456789abcdef")
                        .unwrap(),
                    recipe: None,
                    scope_bases: BTreeMap::new(),
                    project_content_preferences: Default::default(),
                    conversation_checkpoints: BTreeMap::new(),
                    pinned: false,
                    created_at: 1,
                });
                Ok(())
            })
            .unwrap();
        let revision = repository.load_config().unwrap().revision;
        assert_eq!(
            remove_storage(&repository, &storage_id, revision, |_| Ok(()))
                .unwrap_err()
                .code(),
            "storage_linked"
        );
        repository
            .mutate_config(|config| {
                config.links.clear();
                Ok(())
            })
            .unwrap();
        let revision = repository.load_config().unwrap().revision;
        assert_eq!(
            remove_storage(&repository, &storage_id, revision, |_| Err(
                "catalog".to_string()
            ))
            .unwrap_err()
            .code(),
            "storage_removal_rejected"
        );
        remove_storage(&repository, &storage_id, revision, |_| Ok(())).unwrap();
        assert!(repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn draft_operation_pins_the_exact_target_and_prepares_validated_values() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let store_dir = temp.path().join("store");
        let draft = create_storage_setup_draft(&repository, &registry, &test_key()).unwrap();
        let draft = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: draft.revision,
                name: " Proof ".to_string(),
                settings: BoundedConfigObject::empty(),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap();
        assert_eq!(
            begin_storage_draft_operation(
                &repository,
                &draft.draft_id,
                draft.revision + 1,
                "Proof"
            )
            .unwrap_err()
            .code(),
            "storage_setup_draft_revision_conflict"
        );
        assert_eq!(
            begin_storage_draft_operation(&repository, &draft.draft_id, draft.revision, "Other")
                .unwrap_err()
                .code(),
            "storage_setup_draft_name_conflict"
        );
        let operation =
            begin_storage_draft_operation(&repository, &draft.draft_id, draft.revision, "Proof")
                .unwrap();
        assert_eq!(operation.provider(), &test_key());

        let invalid = prepare_storage_draft_in_operation(
            &operation,
            &repository,
            &registry,
            PreparedProviderConfig {
                settings: BoundedConfigObject::empty(),
                secrets: SecretMap::empty(),
            },
        )
        .unwrap_err();
        assert_eq!(invalid.code(), "teststore_settings_invalid");

        let mut secrets = SecretMap::empty();
        secrets
            .insert(token(), SecretValue::parse("minted").unwrap())
            .unwrap();
        let prepared = prepare_storage_draft_in_operation(
            &operation,
            &repository,
            &registry,
            PreparedProviderConfig {
                settings: test_settings(&store_dir),
                secrets,
            },
        )
        .unwrap();
        assert_eq!(prepared.draft_revision, draft.revision + 1);
        let stored = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.storage.name, "Proof");
        assert!(stored.storage.provider_config.secrets.contains(&token()));
        assert!(repository.load_config().unwrap().storages.is_empty());

        // A form edit that landed after the pin wins.
        let concurrent = prepare_storage_draft_in_operation(
            &operation,
            &repository,
            &registry,
            PreparedProviderConfig {
                settings: test_settings(&store_dir),
                secrets: SecretMap::empty(),
            },
        )
        .unwrap_err();
        assert_eq!(concurrent.code(), "storage_setup_draft_revision_conflict");
    }

    #[test]
    fn verify_probes_a_saved_storage_without_mutating_config() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let registry = StorageProviderRegistry::with_test_provider();
        let runtime = runtime();
        let store_dir = temp.path().join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        let storage_id = StorageId::parse("storage-proof").unwrap();
        repository
            .mutate_config(|config| {
                config
                    .storages
                    .push(test_storage_config(storage_id.clone(), "Proof", &store_dir));
                Ok(())
            })
            .unwrap();
        let before = repository.load_config().unwrap();
        let result = verify_storage(&repository, &registry, runtime.handle(), &storage_id).unwrap();
        assert!(result.verified);
        assert_eq!(result.config_revision, before.revision);
        assert_eq!(repository.load_config().unwrap(), before);
        assert_eq!(
            verify_storage(
                &repository,
                StorageProviderRegistry::production(),
                runtime.handle(),
                &storage_id
            )
            .unwrap_err()
            .code(),
            "storage_provider_unavailable"
        );
    }
}

fn storage_settings_locked() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_settings_locked",
        "This storage’s settings are fixed; only its credential can be replaced. To sync to another destination, add a new storage.",
    )
}

/// A committed storage keeps its settings for life: a draft that would
/// write different ones is refused, whichever way it was produced.
fn validate_settings_locked(
    config: &SyncConfig,
    candidate: &StorageConfig,
) -> Result<(), StorageSetupError> {
    if let Some(current) = config.storages.iter().find(|s| s.id == candidate.id) {
        if current.provider_config.settings != candidate.provider_config.settings {
            return Err(storage_settings_locked());
        }
    }
    Ok(())
}

#[cfg(test)]
mod dropbox_tests {
    use super::*;
    use mallard_domain::{SecretMap, SecretValue};
    use mallard_infrastructure::storage_providers::dropbox::{
        provider_key, token_key, DropboxAdapter, DropboxSettingsV1,
    };

    #[test]
    fn dropbox_agent_poc_prepares_only_the_exact_schema_one_draft() {
        // Shared with the HTTP POC. All fixture credentials are synthetic.
        let prepared_config: StorageProviderConfig = serde_json::from_str(include_str!(
            "../../../poc/dropbox-agent-setup/mock-provider-config.json"
        ))
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry = StorageProviderRegistry::production();
        let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let draft = update_storage_setup_draft(
            &repository,
            registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id,
                revision: draft.revision,
                name: "Dropbox POC".into(),
                settings: BoundedConfigObject::empty(),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap();
        assert_eq!(
            begin_storage_draft_operation(
                &repository,
                &draft.draft_id,
                draft.revision + 1,
                "Dropbox POC"
            )
            .unwrap_err()
            .code(),
            "storage_setup_draft_revision_conflict"
        );
        let operation = begin_storage_draft_operation(
            &repository,
            &draft.draft_id,
            draft.revision,
            "Dropbox POC",
        )
        .unwrap();
        assert_eq!(operation.provider(), &provider_key());
        let prepared = prepare_storage_draft_in_operation(
            &operation,
            &repository,
            registry,
            PreparedProviderConfig {
                settings: prepared_config.settings.clone(),
                secrets: prepared_config.secrets.clone(),
            },
        )
        .unwrap();
        assert_eq!(prepared.draft_revision, draft.revision + 1);
        let stored = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.storage.provider_config, prepared_config);
        assert_eq!(stored.storage.provider_config.version, 1);
        assert!(repository.load_config().unwrap().storages.is_empty());
        assert!(DropboxAdapter.descriptor().local_setup == Some(mallard_infrastructure::storage_providers::traits::StorageLocalSetup::Dropbox));
    }

    fn storage(id: &str, namespace: &str, token: &str) -> StorageConfig {
        let mut secrets = SecretMap::empty();
        secrets
            .insert(token_key(), SecretValue::parse(token).unwrap())
            .unwrap();
        StorageConfig {
            id: StorageId::parse(id).unwrap(),
            name: "Dropbox".into(),
            provider: provider_key(),
            provider_config: StorageProviderConfig::new(
                1,
                BoundedConfigObject::encode(&DropboxSettingsV1 {
                    namespace_id: namespace.into(),
                    folder_name: None,
                })
                .unwrap(),
                secrets,
            ),
        }
    }
    #[test]
    fn destination_is_unique_and_namespace_is_immutable_but_token_can_rotate() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let mut config = repository.load_config().unwrap();
        let original = storage("storage-0123456789abcdef01234567", "shared", "token-a");
        config.storages.push(original.clone());
        let rotated = storage(original.id.as_str(), "shared", "token-b");
        assert!(validate_settings_locked(&config, &rotated).is_ok());
        let moved = storage(original.id.as_str(), "different", "token-b");
        assert_eq!(
            validate_settings_locked(&config, &moved)
                .unwrap_err()
                .code(),
            "storage_settings_locked"
        );
        let duplicate = storage("storage-aaaaaaaaaaaaaaaaaaaaaaaa", "shared", "token-b");
        assert_eq!(
            ensure_destination_available(
                StorageProviderRegistry::production(),
                &config,
                &duplicate
            )
            .unwrap_err()
            .code(),
            "storage_destination_conflict"
        );
        assert!(ensure_destination_available(
            StorageProviderRegistry::production(),
            &config,
            &rotated
        )
        .is_ok());
    }
    #[test]
    fn dropbox_mock_setup_commit_token_rotation_and_namespace_rejection() {
        use mallard_infrastructure::storage_core::dropbox_store::mock::MockDropboxServer;
        let server = MockDropboxServer::new();
        let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let draft = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        let settings = BoundedConfigObject::encode(&DropboxSettingsV1 {
            namespace_id: "shared".into(),
            folder_name: None,
        })
        .unwrap();
        let draft = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id,
                revision: draft.revision,
                name: "Shared Dropbox".into(),
                settings: settings.clone(),
                secrets: BTreeMap::from([(
                    token_key(),
                    SecretOperation::Set {
                        value: SecretValue::parse("test-token").unwrap(),
                    },
                )]),
            },
        )
        .unwrap();
        test_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &draft.draft_id,
            draft.revision,
        )
        .unwrap();
        commit_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &draft.draft_id,
            draft.revision,
            |_| Ok(()),
        )
        .unwrap();
        let config = repository.load_config().unwrap();
        let saved = &config.storages[0];
        let identity = continuity_identity(&registry, saved).unwrap();
        assert_eq!(config.schema, 1);
        let edit = create_storage_edit_draft(&repository, &registry, &saved.id).unwrap();
        let edit = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: edit.draft_id,
                revision: edit.revision,
                name: edit.name.clone(),
                settings: settings.clone(),
                secrets: BTreeMap::from([(
                    token_key(),
                    SecretOperation::Replace {
                        value: SecretValue::parse("second-token").unwrap(),
                    },
                )]),
            },
        )
        .unwrap();
        commit_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &edit.draft_id,
            edit.revision,
            |_| Ok(()),
        )
        .unwrap();
        let current = repository.load_config().unwrap();
        assert_eq!(current.storages[0].id, saved.id);
        assert_eq!(
            continuity_identity(&registry, &current.storages[0]).unwrap(),
            identity
        );
        let edit = create_storage_edit_draft(&repository, &registry, &saved.id).unwrap();
        // Moving the namespace is refused at the draft, before any probe.
        let moved = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: edit.draft_id,
                revision: edit.revision,
                name: edit.name.clone(),
                settings: BoundedConfigObject::encode(&DropboxSettingsV1 {
                    namespace_id: "other".into(),
                    folder_name: None,
                })
                .unwrap(),
                secrets: BTreeMap::new(),
            },
        )
        .unwrap_err();
        assert_eq!(moved.code(), "storage_settings_locked");
        assert_eq!(server.probe_file_count(), 0);
    }

    #[test]
    fn summary_carries_runtime_connection_health_and_a_successful_test_forgets_it() {
        use mallard_infrastructure::storage_core::{
            dropbox_store::mock::MockDropboxServer, ConnectionState, ObjectStore,
        };
        let server = MockDropboxServer::new();
        let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
        // Shared stores open on the caller's runtime, as desktop commands do.
        let _runtime = server.runtime().enter();
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let draft = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        let draft = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id,
                revision: draft.revision,
                name: "Shared Dropbox".into(),
                settings: BoundedConfigObject::encode(&DropboxSettingsV1 {
                    namespace_id: "shared".into(),
                    folder_name: None,
                })
                .unwrap(),
                secrets: BTreeMap::from([(
                    token_key(),
                    SecretOperation::Set {
                        value: SecretValue::parse("test-token").unwrap(),
                    },
                )]),
            },
        )
        .unwrap();
        commit_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &draft.draft_id,
            draft.revision,
            |_| Ok(()),
        )
        .unwrap();
        let saved = repository.load_config().unwrap().storages[0].clone();
        // Nothing has been asked in this process: the listing says nothing.
        assert!(storage_summary(&registry, &saved)
            .unwrap()
            .connection
            .is_none());
        let store = ConfiguredObjectStore::share_with(&registry, &saved).unwrap();
        assert_eq!(
            storage_summary(&registry, &saved)
                .unwrap()
                .connection
                .unwrap()
                .state,
            ConnectionState::Connected
        );
        server.set_auth_tag(Some("invalid_access_token"));
        let key = mallard_infrastructure::storage_core::ObjectKey::parse(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_manifests/1-commit-a.json",
        )
        .unwrap();
        assert!(store.get_metadata(&key, 64).is_err());
        let health = storage_summary(&registry, &saved)
            .unwrap()
            .connection
            .unwrap();
        assert_eq!(health.state, ConnectionState::ReconnectRequired);
        assert_eq!(health.code, "dropbox_reconnect_required");
        assert_eq!(health.evidence.unwrap().endpoint, "files/download");
        // Test connection probes with a fresh transport and names the cause.
        let edit = create_storage_edit_draft(&repository, &registry, &saved.id).unwrap();
        let refused = test_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &edit.draft_id,
            edit.revision,
        )
        .unwrap_err();
        assert_eq!(refused.code(), "dropbox_reconnect_required");
        assert!(refused
            .to_string()
            .contains("(users/get_current_account, HTTP 401, invalid_access_token, request req-"));
        assert_eq!(refused.kind(), StorageSetupErrorKind::RemoteFailure);
        // A probe that succeeds forgets the suspended entry.
        server.set_auth_tag(None);
        test_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &edit.draft_id,
            edit.revision,
        )
        .unwrap();
        assert!(storage_summary(&registry, &saved)
            .unwrap()
            .connection
            .is_none());
        assert_eq!(server.probe_file_count(), 0);
    }

    #[test]
    fn managed_credential_lifecycle_survives_reload_and_recovers_by_reconnection() {
        use mallard_infrastructure::dropbox_setup::mock::MockDropboxSetup;
        use mallard_infrastructure::storage_core::{
            dropbox_store::mock::MockDropboxServer, ConnectionState, ObjectKey, ObjectStore,
        };
        use mallard_infrastructure::storage_providers::dropbox::refresh_token_key;
        let server = MockDropboxServer::new();
        let oauth = MockDropboxSetup::new();
        let registry =
            StorageProviderRegistry::from_adapters(vec![server.adapter_with_refresh(&oauth)])
                .unwrap();
        let _runtime = server.runtime().enter();
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let namespace = BoundedConfigObject::encode(&DropboxSettingsV1 {
            namespace_id: "shared".into(),
            folder_name: None,
        })
        .unwrap();
        let prepared = |token: &str| {
            let mut secrets = SecretMap::empty();
            secrets
                .insert(refresh_token_key(), SecretValue::parse(token).unwrap())
                .unwrap();
            PreparedProviderConfig {
                settings: namespace.clone(),
                secrets,
            }
        };
        let key = ObjectKey::parse(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_manifests/1-commit-a.json",
        )
        .unwrap();

        // Setup: the CLI prepares the exact draft; the user finishes it.
        let draft = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        let operation = begin_storage_draft_operation(
            &repository,
            &draft.draft_id,
            draft.revision,
            &draft.name,
        )
        .unwrap();
        prepare_storage_draft_in_operation(
            &operation,
            &repository,
            &registry,
            prepared("refresh-canary"),
        )
        .unwrap();
        drop(operation);
        let draft = get_storage_setup_draft(&repository, &registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        commit_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &draft.draft_id,
            draft.revision,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            oauth.count("refresh"),
            1,
            "the probe's fresh transport refreshed once"
        );
        let saved = repository.load_config().unwrap().storages[0].clone();
        let digest = saved.integrity_digest().unwrap();

        // Restart: a fresh repository over the same directory reads the same
        // credential, and one push renews access once and changes nothing.
        let reloaded = LocalStateRepository::from_home_dir(temp.path())
            .unwrap()
            .load_config()
            .unwrap()
            .storages[0]
            .clone();
        assert_eq!(reloaded.integrity_digest().unwrap(), digest);
        let store = ConfiguredObjectStore::share_with(&registry, &reloaded).unwrap();
        store.put_immutable(&key, b"head").unwrap();
        assert_eq!(oauth.count("refresh"), 2);
        assert_eq!(
            repository.load_config().unwrap().storages[0]
                .integrity_digest()
                .unwrap(),
            digest
        );
        assert_eq!(
            storage_summary(&registry, &reloaded)
                .unwrap()
                .connection
                .unwrap()
                .state,
            ConnectionState::Connected
        );

        // Dropbox stops honouring the refresh token. A fresh transport learns
        // it on its first read, keeps the evidence, and is not asked again.
        ConfiguredObjectStore::forget(&reloaded.id);
        oauth.state.lock().unwrap().deny_refresh = true;
        let error = ConfiguredObjectStore::share_with(&registry, &reloaded)
            .err()
            .expect("a refused refresh fails the first open");
        assert!(
            error.starts_with("Dropbox rejected the saved authorization."),
            "{error}"
        );
        assert!(error.contains("(oauth2/token, HTTP 400, invalid_grant, request req-refresh-1)"));
        assert!(!error.contains("secret-canary") && !error.contains("refresh-canary"));
        assert_eq!(oauth.count("refresh"), 3);
        let health = storage_summary(&registry, &reloaded)
            .unwrap()
            .connection
            .unwrap();
        assert_eq!(health.state, ConnectionState::ReconnectRequired);
        assert_eq!(health.code, "dropbox_reconnect_required");
        let suspended = ConfiguredObjectStore::share_with(&registry, &reloaded).unwrap();
        assert_eq!(suspended.get_metadata(&key, 64).unwrap_err(), error);
        assert_eq!(oauth.count("refresh"), 3, "a suspended store sends nothing");

        // Reconnection: the edit draft takes the token the CLI regained for
        // the same namespace; Save changes probes it fresh and commits.
        let edit = create_storage_edit_draft(&repository, &registry, &reloaded.id).unwrap();
        let operation =
            begin_storage_draft_operation(&repository, &edit.draft_id, edit.revision, &edit.name)
                .unwrap();
        oauth.state.lock().unwrap().deny_refresh = false;
        prepare_storage_draft_in_operation(
            &operation,
            &repository,
            &registry,
            prepared("refresh-renewed"),
        )
        .unwrap();
        drop(operation);
        let edit = get_storage_setup_draft(&repository, &registry, &edit.draft_id)
            .unwrap()
            .unwrap();
        assert!(edit.edits_existing);
        assert!(edit.secrets.iter().all(|secret| secret.value.is_none()));
        commit_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &edit.draft_id,
            edit.revision,
            |_| Ok(()),
        )
        .unwrap();
        let current = repository.load_config().unwrap().storages[0].clone();
        assert_eq!(current.id, reloaded.id);
        assert_ne!(current.integrity_digest().unwrap(), digest);
        assert_eq!(
            continuity_identity(&registry, &current).unwrap(),
            continuity_identity(&registry, &reloaded).unwrap()
        );
        assert!(storage_summary(&registry, &current)
            .unwrap()
            .connection
            .is_none());
        let store = ConfiguredObjectStore::share_with(&registry, &current).unwrap();
        assert_eq!(
            store.get_metadata(&key, 64).unwrap().unwrap().bytes,
            b"head"
        );
        assert_eq!(
            storage_summary(&registry, &current)
                .unwrap()
                .connection
                .unwrap()
                .state,
            ConnectionState::Connected
        );
    }

    #[test]
    fn removal_obeys_the_host_operation_guard() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let _guard = repository.acquire_host_operation().unwrap();
        assert!(remove_storage(
            &repository,
            &StorageId::parse("storage-0123456789abcdef01234567").unwrap(),
            0,
            |_| panic!("must not reach cross-document check")
        )
        .is_err());
    }
}
