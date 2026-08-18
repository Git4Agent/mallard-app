//! Focused storage setup shared by desktop and CLI adapters.
//!
//! Remote verification is completed before the config transaction begins.
//! The transaction then rechecks canonical destination uniqueness while the
//! persistence lock is held and returns the exact committed document.

use mallard_domain::{generated_named_id, StorageConfig, StorageId, StorageKind, SyncConfig};
use mallard_infrastructure::persistence::{
    ConfigMutation, ConfigTransactionError, LocalStateRepository,
};
use mallard_infrastructure::r2_setup_journal::{
    R2SetupId, R2SetupJournalEntry, R2SetupJournalStore, R2SetupJurisdiction, R2SetupStage,
    R2_SETUP_JOURNAL_SCHEMA,
};
use mallard_infrastructure::storage_core::{R2CapabilityProbeErrorKind, S3ObjectStore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const MAX_API_URL_BYTES: usize = 4 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 4 * 1024;

/// A secret-bearing string that zeroizes its allocation on drop. It has no
/// `Debug` or `Serialize` implementation, so adapters cannot accidentally log
/// or emit it through ordinary derived output.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Secret-bearing adapter request. Deliberately does not implement `Debug`,
/// `Clone`, or `Serialize`.
#[derive(Deserialize)]
pub struct ConnectR2StorageRequest {
    pub name: String,
    pub api_url: String,
    pub access_key_id: String,
    pub secret_access_key: SecretString,
    #[serde(default)]
    pub expected_config_revision: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ConnectStorageResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct VerifyStorageResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StorageSummary {
    pub storage_id: StorageId,
    pub name: String,
    pub kind: StorageKind,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageSetupErrorKind {
    InvalidInput,
    Conflict,
    RemoteFailure,
    PersistenceFailure,
    CleanupRequired,
}

impl StorageSetupErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::InvalidInput => 2,
            Self::Conflict => 3,
            Self::RemoteFailure => 4,
            Self::PersistenceFailure => 5,
            Self::CleanupRequired => 6,
        }
    }
}

/// A typed, sanitized adapter error. Messages are selected from fixed text;
/// arbitrary SDK, URL, credential, destination, and filesystem values are
/// never retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageSetupError {
    kind: StorageSetupErrorKind,
    code: &'static str,
    message: &'static str,
    setup_id: Option<String>,
}

impl StorageSetupError {
    pub(crate) fn fixed(
        kind: StorageSetupErrorKind,
        code: &'static str,
        message: &'static str,
    ) -> Self {
        Self {
            kind,
            code,
            message,
            setup_id: None,
        }
    }

    pub(crate) fn invalid(code: &'static str, message: &'static str) -> Self {
        Self::fixed(StorageSetupErrorKind::InvalidInput, code, message)
    }

    pub(crate) fn conflict(code: &'static str, message: &'static str) -> Self {
        Self::fixed(StorageSetupErrorKind::Conflict, code, message)
    }

    pub(crate) fn remote() -> Self {
        Self::fixed(
            StorageSetupErrorKind::RemoteFailure,
            "r2_capability_verification_failed",
            "R2 authentication or required remote capability verification failed.",
        )
    }

    pub(crate) fn persistence() -> Self {
        Self::fixed(
            StorageSetupErrorKind::PersistenceFailure,
            "local_config_unavailable",
            "Mallard local storage configuration is unavailable.",
        )
    }

    pub(crate) fn cleanup_required(setup_id: &str) -> Self {
        Self {
            kind: StorageSetupErrorKind::CleanupRequired,
            code: "r2_probe_cleanup_required",
            message: "R2 remote cleanup is required",
            setup_id: Some(setup_id.to_string()),
        }
    }

    pub fn kind(&self) -> StorageSetupErrorKind {
        self.kind
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn setup_id(&self) -> Option<&str> {
        self.setup_id.as_deref()
    }
}

impl fmt::Display for StorageSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.setup_id {
            Some(setup_id) => write!(formatter, "{} for setup '{}'.", self.message, setup_id),
            None => formatter.write_str(self.message),
        }
    }
}

impl std::error::Error for StorageSetupError {}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum R2Jurisdiction {
    Default,
    Eu,
    Fedramp,
}

impl R2Jurisdiction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Eu => "eu",
            Self::Fedramp => "fedramp",
        }
    }
}

/// Parsed canonical destination. `Debug` and `Serialize` are intentionally
/// omitted because completion and error output must not expose destinations.
#[derive(Clone, PartialEq, Eq)]
pub struct R2Destination {
    jurisdiction: R2Jurisdiction,
    account_id: String,
    bucket: String,
    endpoint: String,
    api_url: String,
}

impl R2Destination {
    pub fn jurisdiction(&self) -> R2Jurisdiction {
        self.jurisdiction
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }
}

/// Parse one strict Cloudflare R2 S3 API URL with exactly one bucket path.
/// Public/custom domains and all URL features that can alias a destination
/// are rejected before canonicalization.
pub fn parse_r2_api_url(value: &str) -> Result<R2Destination, StorageSetupError> {
    if value.is_empty()
        || value.len() > MAX_API_URL_BYTES
        || value.trim() != value
        || value.contains('\\')
    {
        return Err(invalid_api_url());
    }
    let Some((raw_scheme, after_scheme)) = value.split_once("://") else {
        return Err(invalid_api_url());
    };
    if !raw_scheme.eq_ignore_ascii_case("https") {
        return Err(StorageSetupError::invalid(
            "r2_api_url_requires_https",
            "The R2 API URL must use HTTPS.",
        ));
    }
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let raw_authority = &after_scheme[..authority_end];
    let raw_path = &after_scheme[authority_end..];
    if raw_authority.is_empty() || raw_authority.contains('@') || raw_authority.contains(':') {
        return Err(StorageSetupError::invalid(
            "r2_api_url_authority_not_allowed",
            "The R2 API URL must not contain credentials or a port.",
        ));
    }
    if value.contains(['?', '#']) {
        return Err(StorageSetupError::invalid(
            "r2_api_url_suffix_not_allowed",
            "The R2 API URL must not contain a query string or fragment.",
        ));
    }
    let Some(raw_bucket) = raw_path.strip_prefix('/') else {
        return Err(StorageSetupError::invalid(
            "r2_api_url_bucket_required",
            "The R2 API URL must end with exactly one bucket name.",
        ));
    };
    if raw_bucket.is_empty() || raw_bucket.contains(['/', '%']) {
        return Err(StorageSetupError::invalid(
            "r2_api_url_bucket_required",
            "The R2 API URL must end with exactly one bucket name.",
        ));
    }

    let parsed = Url::parse(value).map_err(|_| invalid_api_url())?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid_api_url());
    }
    let host = parsed.host_str().ok_or_else(invalid_api_url)?;
    let labels = host.split('.').collect::<Vec<_>>();
    let (account_id, jurisdiction) = match labels.as_slice() {
        [account, "r2", "cloudflarestorage", "com"] => (*account, R2Jurisdiction::Default),
        [account, "eu", "r2", "cloudflarestorage", "com"] => (*account, R2Jurisdiction::Eu),
        [account, "fedramp", "r2", "cloudflarestorage", "com"] => {
            (*account, R2Jurisdiction::Fedramp)
        }
        _ => {
            return Err(StorageSetupError::invalid(
                "r2_api_url_host_not_supported",
                "The R2 API URL must use a supported Cloudflare R2 S3 endpoint.",
            ))
        }
    };
    if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageSetupError::invalid(
            "r2_account_id_invalid",
            "The R2 API URL account ID must be 32 hexadecimal characters.",
        ));
    }
    if !valid_bucket(raw_bucket) {
        return Err(StorageSetupError::invalid(
            "r2_bucket_name_invalid",
            "The R2 bucket name must use 3 to 63 lowercase letters, numbers, or hyphens and begin and end with a letter or number.",
        ));
    }

    let account_id = account_id.to_ascii_lowercase();
    let endpoint = match jurisdiction {
        R2Jurisdiction::Default => {
            format!("https://{account_id}.r2.cloudflarestorage.com")
        }
        R2Jurisdiction::Eu => {
            format!("https://{account_id}.eu.r2.cloudflarestorage.com")
        }
        R2Jurisdiction::Fedramp => {
            format!("https://{account_id}.fedramp.r2.cloudflarestorage.com")
        }
    };
    Ok(R2Destination {
        jurisdiction,
        account_id,
        bucket: raw_bucket.to_string(),
        api_url: format!("{endpoint}/{raw_bucket}"),
        endpoint,
    })
}

pub fn list_storages(
    repository: &LocalStateRepository,
) -> Result<Vec<StorageSummary>, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    validate_r2_storage_destinations(&config)?;
    Ok(config
        .storages
        .into_iter()
        .map(|storage| StorageSummary {
            storage_id: storage.id,
            name: storage.name,
            kind: storage.kind,
        })
        .collect())
}

/// Validate the clean-cutover invariant for the complete storage document.
/// Every S3 entry must name one canonical Cloudflare R2 destination, and a
/// destination may appear at most once regardless of credentials or display
/// name.
pub fn validate_r2_storage_destinations(config: &SyncConfig) -> Result<(), StorageSetupError> {
    let mut destinations = BTreeSet::new();
    for storage in &config.storages {
        if storage.kind != StorageKind::S3 {
            continue;
        }
        if storage.s3_endpoint.is_empty() || storage.account_id.is_empty() {
            return Err(invalid_stored_destination());
        }
        let destination = parse_r2_api_url(&format!("{}/{}", storage.s3_endpoint, storage.bucket))
            .map_err(|_| invalid_stored_destination())?;
        if storage.s3_endpoint != destination.endpoint()
            || storage.account_id != destination.account_id()
            || storage.bucket != destination.bucket()
        {
            return Err(invalid_stored_destination());
        }
        let key = (
            destination.jurisdiction().as_str(),
            destination.account_id().to_string(),
            destination.bucket().to_string(),
        );
        if !destinations.insert(key) {
            return Err(StorageSetupError::conflict(
                "r2_destination_duplicate",
                "The storage configuration contains the same R2 destination more than once.",
            ));
        }
    }
    Ok(())
}

pub fn connect_r2_storage(
    repository: &LocalStateRepository,
    runtime: Handle,
    request: ConnectR2StorageRequest,
) -> Result<ConnectStorageResult, StorageSetupError> {
    let _operation = acquire_r2_setup_operation(repository)?;
    let probe = ProductionCapabilityProbe { runtime };
    connect_r2_storage_journaled(repository, request, &probe)
}

pub fn verify_storage(
    repository: &LocalStateRepository,
    runtime: Handle,
    storage_id: &StorageId,
) -> Result<VerifyStorageResult, StorageSetupError> {
    let _operation = acquire_r2_setup_operation(repository)?;
    let probe = ProductionCapabilityProbe { runtime };
    verify_storage_journaled(repository, storage_id, &probe)
}

pub(crate) fn acquire_r2_setup_operation(
    repository: &LocalStateRepository,
) -> Result<mallard_infrastructure::persistence::HostOperationGuard, StorageSetupError> {
    repository.acquire_host_operation().map_err(|error| {
        if error == "another host-sensitive operation is already in progress" {
            StorageSetupError::conflict(
                "r2_setup_busy",
                "Another Mallard host operation is already in progress.",
            )
        } else {
            StorageSetupError::persistence()
        }
    })
}

pub(crate) trait CapabilityProbe {
    fn verify(&self, storage: &StorageConfig, setup_id: &str) -> Result<(), StorageSetupError>;

    fn reconcile(
        &self,
        _storage: &StorageConfig,
        _setup_id: &str,
    ) -> Result<(), StorageSetupError> {
        Ok(())
    }
}

pub(crate) struct ProductionCapabilityProbe {
    pub(crate) runtime: Handle,
}

impl CapabilityProbe for ProductionCapabilityProbe {
    fn verify(&self, storage: &StorageConfig, setup_id: &str) -> Result<(), StorageSetupError> {
        let store = S3ObjectStore::from_config(storage, self.runtime.clone())
            .map_err(|_| StorageSetupError::remote())?;
        store
            .probe_r2_capabilities(setup_id)
            .map(|_| ())
            .map_err(|error| match error.kind() {
                R2CapabilityProbeErrorKind::Remote => StorageSetupError::remote(),
                R2CapabilityProbeErrorKind::CleanupRequired => {
                    StorageSetupError::cleanup_required(error.setup_id())
                }
            })
    }

    fn reconcile(&self, storage: &StorageConfig, setup_id: &str) -> Result<(), StorageSetupError> {
        let store = S3ObjectStore::from_config(storage, self.runtime.clone())
            .map_err(|_| StorageSetupError::remote())?;
        store
            .reconcile_r2_capability_probe(setup_id)
            .map_err(|error| match error.kind() {
                R2CapabilityProbeErrorKind::Remote => StorageSetupError::remote(),
                R2CapabilityProbeErrorKind::CleanupRequired => {
                    StorageSetupError::cleanup_required(error.setup_id())
                }
            })
    }
}

fn connect_r2_storage_journaled(
    repository: &LocalStateRepository,
    request: ConnectR2StorageRequest,
    probe: &impl CapabilityProbe,
) -> Result<ConnectStorageResult, StorageSetupError> {
    validate_name(&request.name)?;
    validate_access_key_id(&request.access_key_id)?;
    validate_secret(&request.secret_access_key)?;
    let destination = parse_r2_api_url(&request.api_url)?;
    let mut reconciliation_storage = StorageConfig {
        id: StorageId::parse("storage-r2-reconciliation")
            .map_err(|_| StorageSetupError::persistence())?,
        name: request.name.clone(),
        kind: StorageKind::S3,
        bucket: destination.bucket().to_string(),
        access_key_id: request.access_key_id.clone(),
        secret_access_key: request.secret_access_key.expose().to_string(),
        account_id: destination.account_id().to_string(),
        s3_endpoint: destination.endpoint().to_string(),
        region: "auto".to_string(),
        local_dir: String::new(),
        supports_conditional_writes: Some(true),
    };
    let journal = R2SetupJournalStore::new(repository);
    let reconciliation =
        reconcile_journaled_probes(&journal, probe, &reconciliation_storage, &destination, None);
    reconciliation_storage.secret_access_key.zeroize();
    reconciliation?;

    let (setup_id, mut entry) = start_probe_journal(&journal, &destination)?;
    let result = connect_r2_storage_with_probe_for_setup(
        repository,
        request,
        probe,
        setup_id.as_str().to_string(),
    );
    finish_probe_journal(&journal, &mut entry, result)
}

fn verify_storage_journaled(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    probe: &impl CapabilityProbe,
) -> Result<VerifyStorageResult, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    validate_r2_storage_destinations(&config)?;
    let mut storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(|| {
            StorageSetupError::invalid(
                "storage_not_found",
                "The requested Mallard storage does not exist.",
            )
        })?
        .clone();
    let result = (|| {
        if storage.kind != StorageKind::S3 {
            return Err(StorageSetupError::invalid(
                "storage_not_r2",
                "The requested Mallard storage is not R2 storage.",
            ));
        }
        let destination = destination_from_storage(&storage)?;
        let journal = R2SetupJournalStore::new(repository);
        reconcile_journaled_probes(&journal, probe, &storage, &destination, None)?;
        let (setup_id, mut entry) = start_probe_journal(&journal, &destination)?;
        let result =
            verify_storage_with_probe_for_setup(repository, storage_id, probe, setup_id.as_str());
        finish_probe_journal(&journal, &mut entry, result)
    })();
    storage.secret_access_key.zeroize();
    result
}

fn start_probe_journal(
    journal: &R2SetupJournalStore,
    destination: &R2Destination,
) -> Result<(R2SetupId, R2SetupJournalEntry), StorageSetupError> {
    let setup_id = R2SetupId::parse(
        generated_named_id("r2setup").map_err(|_| StorageSetupError::persistence())?,
    )
    .map_err(|_| StorageSetupError::persistence())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().max(1))
        .unwrap_or(1);
    let entry = R2SetupJournalEntry {
        schema: R2_SETUP_JOURNAL_SCHEMA,
        setup_id: setup_id.clone(),
        account_id: destination.account_id().to_string(),
        bucket: destination.bucket().to_string(),
        jurisdiction: match destination.jurisdiction() {
            R2Jurisdiction::Default => R2SetupJurisdiction::Default,
            R2Jurisdiction::Eu => R2SetupJurisdiction::Eu,
            R2Jurisdiction::Fedramp => R2SetupJurisdiction::Fedramp,
        },
        stage: R2SetupStage::ProbeRunning,
        temporary_token_id: None,
        final_token_id: None,
        cleanup_token_id: None,
        bucket_created: false,
        created_at: timestamp,
        updated_at: timestamp,
        error_code: None,
        probe_cleanup_needed: false,
    };
    journal
        .save(&entry)
        .map_err(|_| StorageSetupError::persistence())?;
    Ok((setup_id, entry))
}

fn finish_probe_journal<T>(
    journal: &R2SetupJournalStore,
    entry: &mut R2SetupJournalEntry,
    result: Result<T, StorageSetupError>,
) -> Result<T, StorageSetupError> {
    match result {
        Ok(value) => {
            journal
                .remove(&entry.setup_id)
                .map_err(|_| StorageSetupError::persistence())?;
            Ok(value)
        }
        Err(error) if error.kind() == StorageSetupErrorKind::CleanupRequired => {
            entry.stage = R2SetupStage::CleanupRequired;
            entry.probe_cleanup_needed = true;
            entry.error_code = Some(error.code().to_string());
            entry.updated_at = entry.updated_at.saturating_add(1);
            let _ = journal.save(entry);
            Err(StorageSetupError::cleanup_required(entry.setup_id.as_str()))
        }
        Err(error) => {
            journal
                .remove(&entry.setup_id)
                .map_err(|_| StorageSetupError::persistence())?;
            Err(error)
        }
    }
}

fn reconcile_journaled_probes(
    journal: &R2SetupJournalStore,
    probe: &impl CapabilityProbe,
    storage: &StorageConfig,
    destination: &R2Destination,
    excluded: Option<&R2SetupId>,
) -> Result<(), StorageSetupError> {
    let jurisdiction = match destination.jurisdiction() {
        R2Jurisdiction::Default => R2SetupJurisdiction::Default,
        R2Jurisdiction::Eu => R2SetupJurisdiction::Eu,
        R2Jurisdiction::Fedramp => R2SetupJurisdiction::Fedramp,
    };
    let entries = journal
        .find_matching(destination.account_id(), jurisdiction, destination.bucket())
        .map_err(|_| StorageSetupError::persistence())?;
    for mut entry in entries {
        if excluded.is_some_and(|excluded| excluded == &entry.setup_id)
            || (!entry.probe_cleanup_needed && entry.stage != R2SetupStage::ProbeRunning)
        {
            continue;
        }
        if probe.reconcile(storage, entry.setup_id.as_str()).is_err() {
            entry.stage = R2SetupStage::CleanupRequired;
            entry.probe_cleanup_needed = true;
            entry.error_code = Some("probe_cleanup_failed".to_string());
            entry.updated_at = entry.updated_at.saturating_add(1);
            let _ = journal.save(&entry);
            return Err(StorageSetupError::cleanup_required(entry.setup_id.as_str()));
        }
        entry.probe_cleanup_needed = false;
        entry.error_code = None;
        if entry.temporary_token_id.is_none()
            && entry.final_token_id.is_none()
            && entry.cleanup_token_id.is_none()
        {
            journal
                .remove(&entry.setup_id)
                .map_err(|_| StorageSetupError::persistence())?;
        } else {
            entry.stage = R2SetupStage::CleanupRequired;
            entry.updated_at = entry.updated_at.saturating_add(1);
            journal
                .save(&entry)
                .map_err(|_| StorageSetupError::persistence())?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn connect_r2_storage_with_probe(
    repository: &LocalStateRepository,
    request: ConnectR2StorageRequest,
    probe: &impl CapabilityProbe,
) -> Result<ConnectStorageResult, StorageSetupError> {
    let setup_id = generated_named_id("r2setup").map_err(|_| StorageSetupError::persistence())?;
    connect_r2_storage_with_probe_for_setup(repository, request, probe, setup_id)
}

pub(crate) fn connect_r2_storage_with_probe_for_setup(
    repository: &LocalStateRepository,
    request: ConnectR2StorageRequest,
    probe: &impl CapabilityProbe,
    setup_id: String,
) -> Result<ConnectStorageResult, StorageSetupError> {
    validate_name(&request.name)?;
    validate_access_key_id(&request.access_key_id)?;
    validate_secret(&request.secret_access_key)?;
    let destination = parse_r2_api_url(&request.api_url)?;
    let storage_id = StorageId::parse(
        generated_named_id("storage").map_err(|_| StorageSetupError::persistence())?,
    )
    .map_err(|_| StorageSetupError::persistence())?;
    let pending = PendingStorage::new(StorageConfig {
        id: storage_id,
        name: request.name,
        kind: StorageKind::S3,
        bucket: destination.bucket().to_string(),
        access_key_id: request.access_key_id,
        secret_access_key: request.secret_access_key.expose().to_string(),
        account_id: destination.account_id().to_string(),
        s3_endpoint: destination.endpoint().to_string(),
        region: "auto".to_string(),
        local_dir: String::new(),
        supports_conditional_writes: Some(true),
    });
    pending
        .storage()
        .validate()
        .map_err(|_| invalid_request())?;

    let before = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    validate_r2_storage_destinations(&before)?;
    let preflight = destination_status(&before, &destination, pending.storage())?;
    if matches!(preflight, DestinationStatus::Conflict) {
        return Err(duplicate_destination());
    }
    if !matches!(preflight, DestinationStatus::Exact(_))
        && request
            .expected_config_revision
            .is_some_and(|expected| expected != before.revision)
    {
        return Err(revision_conflict());
    }

    probe.verify(pending.storage(), &setup_id)?;

    let expected_revision = request.expected_config_revision;
    let destination_for_commit = destination.clone();
    let ((storage_id, _changed), committed) = repository
        .transact_config(move |config| {
            validate_r2_storage_destinations(config)?;
            match destination_status(config, &destination_for_commit, pending.storage())? {
                DestinationStatus::Exact(storage_id) => {
                    Ok(ConfigMutation::Unchanged((storage_id, false)))
                }
                DestinationStatus::Conflict => Err(duplicate_destination()),
                DestinationStatus::Missing => {
                    if expected_revision.is_some_and(|expected| expected != config.revision) {
                        return Err(revision_conflict());
                    }
                    let storage_id = pending.storage().id.clone();
                    config.storages.push(pending.into_storage());
                    validate_r2_storage_destinations(config)?;
                    Ok(ConfigMutation::Changed((storage_id, true)))
                }
            }
        })
        .map_err(|error| match error {
            ConfigTransactionError::Mutation(error) => error,
            ConfigTransactionError::Persistence(_) => StorageSetupError::persistence(),
        })?;

    Ok(ConnectStorageResult {
        storage_id,
        config_revision: committed.revision,
        verified: true,
    })
}

#[cfg(test)]
fn verify_storage_with_probe(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    probe: &impl CapabilityProbe,
) -> Result<VerifyStorageResult, StorageSetupError> {
    let setup_id = generated_named_id("r2setup").map_err(|_| StorageSetupError::persistence())?;
    verify_storage_with_probe_for_setup(repository, storage_id, probe, &setup_id)
}

fn verify_storage_with_probe_for_setup(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    probe: &impl CapabilityProbe,
    setup_id: &str,
) -> Result<VerifyStorageResult, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    validate_r2_storage_destinations(&config)?;
    let storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(|| {
            StorageSetupError::invalid(
                "storage_not_found",
                "The requested Mallard storage does not exist.",
            )
        })?;
    if storage.kind != StorageKind::S3 {
        return Err(StorageSetupError::invalid(
            "storage_not_r2",
            "The requested Mallard storage is not R2 storage.",
        ));
    }
    destination_from_storage(storage)?;
    probe.verify(storage, setup_id)?;
    Ok(VerifyStorageResult {
        storage_id: storage.id.clone(),
        config_revision: config.revision,
        verified: true,
    })
}

enum DestinationStatus {
    Missing,
    Exact(StorageId),
    Conflict,
}

fn destination_status(
    config: &SyncConfig,
    destination: &R2Destination,
    pending: &StorageConfig,
) -> Result<DestinationStatus, StorageSetupError> {
    let mut matched: Option<&StorageConfig> = None;
    for storage in &config.storages {
        if storage.kind != StorageKind::S3 {
            continue;
        }
        let stored_destination = destination_from_storage(storage)?;
        if &stored_destination == destination {
            if matched.is_some() {
                return Ok(DestinationStatus::Conflict);
            }
            matched = Some(storage);
        }
    }
    match matched {
        None => Ok(DestinationStatus::Missing),
        Some(existing) if same_setup(existing, pending) => {
            Ok(DestinationStatus::Exact(existing.id.clone()))
        }
        Some(_) => Ok(DestinationStatus::Conflict),
    }
}

fn destination_from_storage(storage: &StorageConfig) -> Result<R2Destination, StorageSetupError> {
    if storage.s3_endpoint.is_empty() {
        return Err(StorageSetupError::persistence());
    }
    let endpoint = storage.s3_endpoint.clone();
    let destination = parse_r2_api_url(&format!("{endpoint}/{}", storage.bucket))
        .map_err(|_| StorageSetupError::persistence())?;
    if !storage.account_id.is_empty()
        && !storage
            .account_id
            .eq_ignore_ascii_case(destination.account_id())
    {
        return Err(StorageSetupError::persistence());
    }
    Ok(destination)
}

fn same_setup(existing: &StorageConfig, pending: &StorageConfig) -> bool {
    existing.name == pending.name
        && existing.kind == pending.kind
        && existing.bucket == pending.bucket
        && existing.access_key_id == pending.access_key_id
        && existing.secret_access_key == pending.secret_access_key
        && existing.account_id == pending.account_id
        && existing.s3_endpoint == pending.s3_endpoint
        && existing.region == pending.region
        && existing.local_dir == pending.local_dir
        && existing.supports_conditional_writes == pending.supports_conditional_writes
}

struct PendingStorage(Option<StorageConfig>);

impl PendingStorage {
    fn new(storage: StorageConfig) -> Self {
        Self(Some(storage))
    }

    fn storage(&self) -> &StorageConfig {
        self.0.as_ref().expect("pending storage is present")
    }

    fn into_storage(mut self) -> StorageConfig {
        self.0.take().expect("pending storage is present")
    }
}

impl Drop for PendingStorage {
    fn drop(&mut self) {
        if let Some(storage) = &mut self.0 {
            storage.secret_access_key.zeroize();
        }
    }
}

fn validate_name(name: &str) -> Result<(), StorageSetupError> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 1_024
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

fn validate_access_key_id(value: &str) -> Result<(), StorageSetupError> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(StorageSetupError::invalid(
            "r2_access_key_id_invalid",
            "The R2 Access Key ID is invalid.",
        ))
    } else {
        Ok(())
    }
}

fn validate_secret(secret: &SecretString) -> Result<(), StorageSetupError> {
    if secret.expose().is_empty()
        || secret.expose().len() > MAX_CREDENTIAL_BYTES
        || secret.expose().chars().any(char::is_control)
    {
        Err(StorageSetupError::invalid(
            "r2_secret_access_key_invalid",
            "The R2 Secret Access Key is invalid.",
        ))
    } else {
        Ok(())
    }
}

fn valid_bucket(bucket: &str) -> bool {
    (3..=63).contains(&bucket.len())
        && bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && bucket
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && bucket
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn invalid_api_url() -> StorageSetupError {
    StorageSetupError::invalid("r2_api_url_invalid", "The R2 API URL is invalid.")
}

fn invalid_request() -> StorageSetupError {
    StorageSetupError::invalid("r2_storage_invalid", "The R2 storage settings are invalid.")
}

fn invalid_stored_destination() -> StorageSetupError {
    StorageSetupError::invalid(
        "r2_storage_destination_invalid",
        "Every S3 storage must use one canonical Cloudflare R2 destination.",
    )
}

fn duplicate_destination() -> StorageSetupError {
    StorageSetupError::conflict(
        "r2_destination_conflict",
        "That R2 destination is already connected with different settings.",
    )
}

fn revision_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "config_revision_conflict",
        "Mallard storage configuration changed; reload it before connecting storage.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::{Arc, Barrier};

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
    const ACCESS: &str = "access-key-canary";
    const SECRET: &str = "secret-canary";

    struct FakeProbe {
        calls: Cell<usize>,
        outcome: RefCell<Option<StorageSetupError>>,
        reconcile_calls: Cell<usize>,
        reconcile_outcome: RefCell<Option<StorageSetupError>>,
    }

    impl FakeProbe {
        fn successful() -> Self {
            Self {
                calls: Cell::new(0),
                outcome: RefCell::new(None),
                reconcile_calls: Cell::new(0),
                reconcile_outcome: RefCell::new(None),
            }
        }
    }

    impl CapabilityProbe for FakeProbe {
        fn verify(
            &self,
            _storage: &StorageConfig,
            _setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.calls.set(self.calls.get() + 1);
            match self.outcome.borrow().as_ref() {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn reconcile(
            &self,
            _storage: &StorageConfig,
            _setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.reconcile_calls.set(self.reconcile_calls.get() + 1);
            match self.reconcile_outcome.borrow().as_ref() {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
    }

    struct BarrierProbe(Arc<Barrier>);

    impl CapabilityProbe for BarrierProbe {
        fn verify(
            &self,
            _storage: &StorageConfig,
            _setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.0.wait();
            Ok(())
        }
    }

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path()).unwrap()
    }

    fn request(
        name: &str,
        api_url: &str,
        access_key_id: &str,
        secret: &str,
        expected_config_revision: Option<u64>,
    ) -> ConnectR2StorageRequest {
        ConnectR2StorageRequest {
            name: name.to_string(),
            api_url: api_url.to_string(),
            access_key_id: access_key_id.to_string(),
            secret_access_key: SecretString::new(secret.to_string()),
            expected_config_revision,
        }
    }

    fn default_url(bucket: &str) -> String {
        format!("https://{ACCOUNT}.r2.cloudflarestorage.com/{bucket}")
    }

    #[test]
    fn parser_canonicalizes_all_supported_r2_jurisdictions() {
        for (url, jurisdiction, endpoint) in [
            (
                format!(
                    "https://{}.r2.cloudflarestorage.com/team-backup",
                    ACCOUNT.to_uppercase()
                ),
                R2Jurisdiction::Default,
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
            ),
            (
                format!("https://{ACCOUNT}.eu.r2.cloudflarestorage.com/team-backup"),
                R2Jurisdiction::Eu,
                format!("https://{ACCOUNT}.eu.r2.cloudflarestorage.com"),
            ),
            (
                format!("https://{ACCOUNT}.fedramp.r2.cloudflarestorage.com/team-backup"),
                R2Jurisdiction::Fedramp,
                format!("https://{ACCOUNT}.fedramp.r2.cloudflarestorage.com"),
            ),
        ] {
            let parsed = parse_r2_api_url(&url).unwrap();
            assert_eq!(parsed.jurisdiction(), jurisdiction);
            assert_eq!(parsed.account_id(), ACCOUNT);
            assert_eq!(parsed.bucket(), "team-backup");
            assert_eq!(parsed.endpoint(), endpoint);
            assert_eq!(parsed.api_url(), format!("{endpoint}/team-backup"));
        }
    }

    #[test]
    fn parser_rejects_aliasing_and_unsupported_url_forms() {
        for value in [
            format!("http://{ACCOUNT}.r2.cloudflarestorage.com/bucket"),
            format!("https://user@{ACCOUNT}.r2.cloudflarestorage.com/bucket"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com:443/bucket"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/bucket?x=1"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/bucket#fragment"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/one/two"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/Bucket"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/-bucket"),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/bu"),
            format!("https://{ACCOUNT}.r2.dev/bucket"),
            "https://short.r2.cloudflarestorage.com/bucket".to_string(),
        ] {
            let error = parse_r2_api_url(&value)
                .err()
                .expect("URL must be rejected");
            assert_eq!(error.kind(), StorageSetupErrorKind::InvalidInput);
            let display = error.to_string();
            assert!(!display.contains(ACCOUNT));
            assert!(!display.contains("team-backup"));
        }
    }

    #[test]
    fn connect_is_idempotent_and_returns_the_exact_committed_revision() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();
        let url = default_url("team-backup");

        let first = connect_r2_storage_with_probe(
            &repository,
            request("Team backup", &url, ACCESS, SECRET, Some(0)),
            &probe,
        )
        .unwrap();
        assert_eq!(first.config_revision, 1);
        assert!(first.verified);

        let retry = connect_r2_storage_with_probe(
            &repository,
            request("Team backup", &url, ACCESS, SECRET, Some(0)),
            &probe,
        )
        .unwrap();
        assert_eq!(retry.storage_id, first.storage_id);
        assert_eq!(retry.config_revision, 1);
        assert_eq!(repository.load_config().unwrap().revision, 1);
        assert_eq!(repository.load_config().unwrap().storages.len(), 1);
        assert_eq!(probe.calls.get(), 2);
    }

    #[test]
    fn conflicting_duplicate_and_stale_revision_fail_before_remote_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();
        let url = default_url("team-backup");
        connect_r2_storage_with_probe(
            &repository,
            request("Team backup", &url, ACCESS, SECRET, None),
            &probe,
        )
        .unwrap();
        assert_eq!(probe.calls.get(), 1);

        let duplicate = connect_r2_storage_with_probe(
            &repository,
            request("Other name", &url, ACCESS, SECRET, None),
            &probe,
        )
        .unwrap_err();
        assert_eq!(duplicate.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(probe.calls.get(), 1);

        let stale = connect_r2_storage_with_probe(
            &repository,
            request(
                "Another",
                &default_url("another-bucket"),
                ACCESS,
                SECRET,
                Some(0),
            ),
            &probe,
        )
        .unwrap_err();
        assert_eq!(stale.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(probe.calls.get(), 1);
        assert_eq!(repository.load_config().unwrap().storages.len(), 1);
    }

    #[test]
    fn concurrent_connects_recheck_destination_uniqueness_under_the_config_lock() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let barrier = Arc::new(Barrier::new(2));
        let url = default_url("team-backup");
        let first_repository = repository.clone();
        let first_barrier = barrier.clone();
        let first_url = url.clone();
        let first = std::thread::spawn(move || {
            connect_r2_storage_with_probe(
                &first_repository,
                request("First", &first_url, ACCESS, SECRET, None),
                &BarrierProbe(first_barrier),
            )
        });
        let second_repository = repository.clone();
        let second = std::thread::spawn(move || {
            connect_r2_storage_with_probe(
                &second_repository,
                request("Second", &url, ACCESS, SECRET, None),
                &BarrierProbe(barrier),
            )
        });

        let outcomes = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        let conflict = outcomes
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one connect conflicts");
        assert_eq!(conflict.kind(), StorageSetupErrorKind::Conflict);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages.len(), 1);
        assert_eq!(config.revision, 1);
    }

    #[test]
    fn remote_failure_never_commits_config_or_exposes_canaries() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe {
            calls: Cell::new(0),
            outcome: RefCell::new(Some(StorageSetupError::remote())),
            reconcile_calls: Cell::new(0),
            reconcile_outcome: RefCell::new(None),
        };
        let error = connect_r2_storage_with_probe(
            &repository,
            request(
                "Team backup",
                &default_url("private-bucket"),
                ACCESS,
                SECRET,
                None,
            ),
            &probe,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::RemoteFailure);
        let output = format!("{error:?} {error}");
        for canary in [ACCOUNT, "private-bucket", ACCESS, SECRET] {
            assert!(!output.contains(canary), "leaked {canary}");
        }
        assert!(repository.load_config().unwrap().storages.is_empty());
        assert_eq!(repository.load_config().unwrap().revision, 0);
    }

    #[test]
    fn existing_credential_connect_persists_the_exact_s3_secret() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();
        let supplied = "cfat_value-that-is-explicitly-an-s3-secret";
        connect_r2_storage_with_probe(
            &repository,
            request(
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                supplied,
                None,
            ),
            &probe,
        )
        .unwrap();

        let stored = &repository.load_config().unwrap().storages[0].secret_access_key;
        assert_eq!(stored, supplied);
    }

    #[test]
    fn verify_runs_the_probe_without_mutating_config() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let connect_probe = FakeProbe::successful();
        let connected = connect_r2_storage_with_probe(
            &repository,
            request(
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                SECRET,
                None,
            ),
            &connect_probe,
        )
        .unwrap();
        let verify_probe = FakeProbe::successful();

        let verified =
            verify_storage_with_probe(&repository, &connected.storage_id, &verify_probe).unwrap();
        assert_eq!(verified.storage_id, connected.storage_id);
        assert_eq!(verified.config_revision, connected.config_revision);
        assert!(verified.verified);
        assert_eq!(verify_probe.calls.get(), 1);
        assert_eq!(repository.load_config().unwrap().revision, 1);
    }

    #[test]
    fn journaled_connect_reconciles_an_interrupted_probe_before_retry() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();
        *probe.outcome.borrow_mut() = Some(StorageSetupError::cleanup_required(
            "r2setup-111111111111111111111111",
        ));

        let first = connect_r2_storage_journaled(
            &repository,
            request(
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                SECRET,
                Some(0),
            ),
            &probe,
        )
        .unwrap_err();
        assert_eq!(first.kind(), StorageSetupErrorKind::CleanupRequired);
        assert_eq!(
            R2SetupJournalStore::new(&repository).list().unwrap().len(),
            1
        );
        assert!(repository.load_config().unwrap().storages.is_empty());

        *probe.outcome.borrow_mut() = None;
        let retry = connect_r2_storage_journaled(
            &repository,
            request(
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                SECRET,
                Some(0),
            ),
            &probe,
        )
        .unwrap();
        assert!(retry.verified);
        assert_eq!(probe.reconcile_calls.get(), 1);
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
        assert_eq!(repository.load_config().unwrap().storages.len(), 1);
    }

    #[test]
    fn r2_setup_operation_reports_busy_without_exposing_lock_details() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let guard = repository.acquire_host_operation().unwrap();

        let error = acquire_r2_setup_operation(&repository).unwrap_err();
        assert_eq!(error.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(error.code(), "r2_setup_busy");
        assert_eq!(
            error.to_string(),
            "Another Mallard host operation is already in progress."
        );

        drop(guard);
        acquire_r2_setup_operation(&repository).unwrap();
    }

    fn stored_r2(id: &str, endpoint: String, account_id: String, bucket: &str) -> StorageConfig {
        StorageConfig {
            id: StorageId::parse(id).unwrap(),
            name: id.to_string(),
            kind: StorageKind::S3,
            bucket: bucket.to_string(),
            access_key_id: format!("access-{id}"),
            secret_access_key: format!("secret-{id}"),
            account_id,
            s3_endpoint: endpoint,
            region: "auto".to_string(),
            local_dir: String::new(),
            supports_conditional_writes: Some(true),
        }
    }

    #[test]
    fn full_document_validator_rejects_noncanonical_and_generic_s3_entries() {
        for storage in [
            stored_r2(
                "storage-uppercase-account",
                format!(
                    "https://{}.r2.cloudflarestorage.com",
                    ACCOUNT.to_uppercase()
                ),
                ACCOUNT.to_uppercase(),
                "team-backup",
            ),
            stored_r2(
                "storage-missing-account",
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
                String::new(),
                "team-backup",
            ),
            stored_r2(
                "storage-generic-s3",
                "https://s3.example.com".to_string(),
                ACCOUNT.to_string(),
                "team-backup",
            ),
        ] {
            let mut config = SyncConfig::default();
            config.storages.push(storage);
            let error = validate_r2_storage_destinations(&config).unwrap_err();
            assert_eq!(error.kind(), StorageSetupErrorKind::InvalidInput);
            assert_eq!(error.code(), "r2_storage_destination_invalid");
            assert!(!error.to_string().contains(ACCOUNT));
            assert!(!error.to_string().contains("team-backup"));
        }
    }

    #[test]
    fn full_document_validator_rejects_duplicate_canonical_destinations() {
        let endpoint = format!("https://{ACCOUNT}.r2.cloudflarestorage.com");
        let mut config = SyncConfig::default();
        config.storages.push(stored_r2(
            "storage-first",
            endpoint.clone(),
            ACCOUNT.to_string(),
            "team-backup",
        ));
        config.storages.push(stored_r2(
            "storage-second",
            endpoint,
            ACCOUNT.to_string(),
            "team-backup",
        ));

        let error = validate_r2_storage_destinations(&config).unwrap_err();
        assert_eq!(error.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(error.code(), "r2_destination_duplicate");
    }

    #[test]
    fn full_document_validator_allows_distinct_canonical_jurisdictions() {
        let mut config = SyncConfig::default();
        config.storages.push(stored_r2(
            "storage-default",
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
            ACCOUNT.to_string(),
            "team-backup",
        ));
        config.storages.push(stored_r2(
            "storage-eu",
            format!("https://{ACCOUNT}.eu.r2.cloudflarestorage.com"),
            ACCOUNT.to_string(),
            "team-backup",
        ));
        config.storages.push(stored_r2(
            "storage-other-bucket",
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
            ACCOUNT.to_string(),
            "other-bucket",
        ));

        validate_r2_storage_destinations(&config).unwrap();
    }

    #[test]
    fn invalid_existing_s3_entry_blocks_connect_and_verify_before_remote_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let invalid = stored_r2(
            "storage-invalid-existing",
            format!(
                "https://{}.r2.cloudflarestorage.com",
                ACCOUNT.to_uppercase()
            ),
            ACCOUNT.to_uppercase(),
            "team-backup",
        );
        let invalid_id = invalid.id.clone();
        let mut config = SyncConfig::default();
        config.storages.push(invalid);
        repository.save_config(config).unwrap();
        let probe = FakeProbe::successful();

        let connect_error = connect_r2_storage_with_probe(
            &repository,
            request(
                "Other bucket",
                &default_url("other-bucket"),
                ACCESS,
                SECRET,
                None,
            ),
            &probe,
        )
        .unwrap_err();
        assert_eq!(connect_error.code(), "r2_storage_destination_invalid");
        let verify_error = verify_storage_with_probe(&repository, &invalid_id, &probe).unwrap_err();
        assert_eq!(verify_error.code(), "r2_storage_destination_invalid");
        assert_eq!(probe.calls.get(), 0);
    }
}
