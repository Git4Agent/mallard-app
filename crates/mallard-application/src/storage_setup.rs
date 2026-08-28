//! Focused storage setup shared by desktop and CLI adapters.
//!
//! Remote verification is completed before the config transaction begins.
//! The transaction then rechecks canonical destination uniqueness while the
//! persistence lock is held.

use mallard_domain::{
    generated_named_id, StorageConfig, StorageId, StorageKind, StorageSetupDraft,
    StorageSetupDraftId, SyncConfig,
};
use mallard_infrastructure::persistence::{
    ConfigMutation, ConfigTransactionError, LocalStateRepository,
};
use mallard_infrastructure::r2_setup_journal::{
    R2SetupId, R2SetupJournalEntry, R2SetupJournalStore, R2SetupJurisdiction, R2SetupStage,
    R2_SETUP_JOURNAL_SCHEMA,
};
use mallard_infrastructure::storage_core::{R2CapabilityProbeErrorKind, S3ObjectStore};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
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

/// Browser-minted values to place into an existing setup draft. Deliberately
/// does not implement `Debug`, `Clone`, or `Serialize`.
pub struct PrepareR2StorageDraftRequest {
    pub api_url: String,
    pub access_key_id: String,
    pub secret_access_key: SecretString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepareR2StorageDraftResult {
    pub draft_id: StorageSetupDraftId,
    pub draft_revision: u64,
}

/// Secret-bearing request for one desktop Create, Edit, or Test operation. A
/// missing secret is accepted when `storage_id` identifies an existing R2
/// storage (the stored backend secret is preserved) or when `draft_id`
/// identifies a setup draft holding one; naming both is invalid.
pub struct UpsertR2StorageRequest {
    pub storage_id: Option<StorageId>,
    /// Setup draft to resolve a blank secret from. Draft references are an
    /// all-or-none triple pinned to the exact revision and inert storage
    /// identity reviewed by the UI. A successful upsert commit also
    /// best-effort discards the draft it consumed.
    pub draft_id: Option<StorageSetupDraftId>,
    pub draft_revision: Option<u64>,
    pub draft_storage_id: Option<StorageId>,
    pub name: String,
    pub api_url: String,
    pub access_key_id: String,
    pub secret_access_key: Option<SecretString>,
    pub expected_config_revision: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UpsertR2StorageResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TestR2StorageResult {
    /// The revision the request was checked against — nothing was committed.
    pub config_revision: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyStorageResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageSummary {
    pub storage_id: StorageId,
    pub name: String,
    pub kind: StorageKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
            "Mallard could not access its local storage configuration. If this command is running in a restricted environment, approve read/write access to Mallard's local state and run the same command again.",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Exclusive host-operation token for preparing one existing R2 setup draft.
/// The CLI keeps this value alive across Wrangler authentication and browser
/// consent. The draft revision is pinned before any Cloudflare work so a
/// concurrent form edit cannot be overwritten when authorization completes.
pub struct R2StorageDraftOperation {
    repository_root: PathBuf,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    name: String,
    _guard: mallard_infrastructure::persistence::HostOperationGuard,
}

/// Validate and pin the exact editable draft before making any Cloudflare or
/// browser request.
pub fn begin_r2_storage_draft_operation(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
    expected_revision: u64,
    name: &str,
) -> Result<R2StorageDraftOperation, StorageSetupError> {
    validate_name(name)?;
    let guard = acquire_r2_setup_operation(repository)?;
    let mut draft = load_storage_setup_draft(repository, draft_id)?;
    let validation = validate_storage_setup_draft_target(&draft, expected_revision, name);
    draft.storage.secret_access_key.zeroize();
    validation?;
    Ok(R2StorageDraftOperation {
        repository_root: repository.root().to_path_buf(),
        draft_id: draft_id.clone(),
        expected_revision,
        name: name.to_string(),
        _guard: guard,
    })
}

/// Put browser-minted values into the pinned draft without probing remote
/// capabilities or mutating the committed storage configuration. The user
/// reviews, optionally tests, and explicitly finishes setup in Mallard.
pub fn prepare_r2_storage_draft_in_operation(
    operation: &R2StorageDraftOperation,
    repository: &LocalStateRepository,
    request: PrepareR2StorageDraftRequest,
) -> Result<PrepareR2StorageDraftResult, StorageSetupError> {
    ensure_operation_repository(operation, repository)?;
    validate_access_key_id(&request.access_key_id)?;
    validate_secret(&request.secret_access_key)?;
    let destination = parse_r2_api_url(&request.api_url)?;
    let mut draft = load_storage_setup_draft(repository, &operation.draft_id)?;
    let validation =
        validate_storage_setup_draft_target(&draft, operation.expected_revision, &operation.name);
    draft.storage.secret_access_key.zeroize();
    validation?;
    draft.storage.name = operation.name.clone();
    draft.storage.kind = StorageKind::S3;
    draft.storage.bucket = destination.bucket().to_string();
    draft.storage.access_key_id = request.access_key_id;
    draft.storage.secret_access_key = request.secret_access_key.expose().to_string();
    draft.storage.account_id = destination.account_id().to_string();
    draft.storage.s3_endpoint = destination.endpoint().to_string();
    draft.storage.region = "auto".to_string();
    draft.storage.local_dir.clear();
    // The CLI only prepares the form. The user's Test connection or Finish
    // action is what establishes this capability.
    draft.storage.supports_conditional_writes = None;
    draft.updated_at = unix_timestamp().max(draft.updated_at);

    let mut saved = repository
        .save_storage_setup_draft(draft)
        .map_err(map_storage_setup_draft_save_error)?;
    let result = PrepareR2StorageDraftResult {
        draft_id: saved.draft_id.clone(),
        draft_revision: saved.revision,
    };
    saved.storage.secret_access_key.zeroize();
    Ok(result)
}

/// Verify and atomically create or replace one schema-1 R2 storage. The host
/// operation remains held across journal reconciliation, remote probing, and
/// the revision-guarded config transaction.
pub fn upsert_r2_storage(
    repository: &LocalStateRepository,
    runtime: Handle,
    request: UpsertR2StorageRequest,
) -> Result<UpsertR2StorageResult, StorageSetupError> {
    let probe = ProductionCapabilityProbe { runtime };
    upsert_r2_storage_with_probe(repository, request, &probe)
}

/// Verify form-entered R2 settings against the live bucket without committing
/// anything: the exact upsert pipeline — validation, exclusive host
/// operation, journal reconciliation, and the journaled capability probe —
/// minus the config transaction. A test never discards a referenced draft.
pub fn test_r2_storage(
    repository: &LocalStateRepository,
    runtime: Handle,
    request: UpsertR2StorageRequest,
) -> Result<TestR2StorageResult, StorageSetupError> {
    let probe = ProductionCapabilityProbe { runtime };
    test_r2_storage_with_probe(repository, request, &probe)
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

fn acquire_r2_setup_operation(
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

fn ensure_operation_repository(
    operation: &R2StorageDraftOperation,
    repository: &LocalStateRepository,
) -> Result<(), StorageSetupError> {
    if operation.repository_root == repository.root() {
        Ok(())
    } else {
        Err(StorageSetupError::persistence())
    }
}

fn load_storage_setup_draft(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
) -> Result<StorageSetupDraft, StorageSetupError> {
    repository
        .load_storage_setup_draft(draft_id)
        .map_err(|_| StorageSetupError::persistence())?
        .ok_or_else(storage_setup_draft_not_found)
}

fn validate_storage_setup_draft_target(
    draft: &StorageSetupDraft,
    expected_revision: u64,
    name: &str,
) -> Result<(), StorageSetupError> {
    if draft.revision != expected_revision {
        return Err(storage_setup_draft_revision_conflict());
    }
    if draft.storage.kind != StorageKind::S3 {
        return Err(storage_setup_draft_not_r2());
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
    } else {
        StorageSetupError::persistence()
    }
}

trait CapabilityProbe {
    fn verify(&self, storage: &StorageConfig, setup_id: &str) -> Result<(), StorageSetupError>;

    fn reconcile(
        &self,
        _storage: &StorageConfig,
        _setup_id: &str,
    ) -> Result<(), StorageSetupError> {
        Ok(())
    }
}

struct ProductionCapabilityProbe {
    runtime: Handle,
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

/// Everything one verified upsert or test shares before remote probing. Owns
/// the exclusive host guard and every secret copy: `existing` and `pending`
/// zeroize on drop.
struct PreparedR2Upsert {
    _operation: mallard_infrastructure::persistence::HostOperationGuard,
    destination: R2Destination,
    storage_id: StorageId,
    target_storage_id: Option<StorageId>,
    expected_config_revision: u64,
    existing: Option<PendingStorage>,
    pending: PendingStorage,
}

/// The verified payload handed to a caller once the remote probe has passed.
struct VerifiedR2Upsert {
    storage_id: StorageId,
    target_storage_id: Option<StorageId>,
    destination: R2Destination,
    expected_config_revision: u64,
    pending: PendingStorage,
}

fn prepare_r2_upsert(
    repository: &LocalStateRepository,
    request: UpsertR2StorageRequest,
) -> Result<PreparedR2Upsert, StorageSetupError> {
    let has_any_draft_reference = request.draft_id.is_some()
        || request.draft_revision.is_some()
        || request.draft_storage_id.is_some();
    if request.storage_id.is_some() && has_any_draft_reference {
        return Err(StorageSetupError::invalid(
            "storage_draft_request_invalid",
            "A storage request must not name both an existing storage and a setup draft.",
        ));
    }
    let draft_reference = match (
        request.draft_id.as_ref(),
        request.draft_revision,
        request.draft_storage_id.as_ref(),
    ) {
        (None, None, None) => None,
        (Some(draft_id), Some(draft_revision), Some(draft_storage_id)) => {
            Some((draft_id, draft_revision, draft_storage_id))
        }
        _ => {
            return Err(StorageSetupError::invalid(
                "storage_draft_request_invalid",
                "A storage setup draft reference is incomplete.",
            ));
        }
    };
    validate_name(&request.name)?;
    validate_access_key_id(&request.access_key_id)?;
    if let Some(secret) = &request.secret_access_key {
        validate_secret(secret)?;
    }
    let destination = parse_r2_api_url(&request.api_url)?;
    let operation = acquire_r2_setup_operation(repository)?;

    let before = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    if before.revision != request.expected_config_revision {
        return Err(revision_conflict());
    }
    validate_r2_storage_destinations(&before)?;

    let existing = match request.storage_id.as_ref() {
        Some(storage_id) => {
            let storage = before
                .storages
                .iter()
                .find(|storage| &storage.id == storage_id)
                .ok_or_else(storage_not_found)?;
            if storage.kind != StorageKind::S3 {
                return Err(storage_not_r2());
            }
            Some(PendingStorage::new(storage.clone()))
        }
        None => None,
    };
    let mut draft = draft_reference
        .map(|(draft_id, draft_revision, draft_storage_id)| {
            load_pinned_r2_storage_setup_draft(
                repository,
                draft_id,
                draft_revision,
                draft_storage_id,
            )
        })
        .transpose()?;
    if let Some(draft) = draft.as_ref() {
        validate_pinned_r2_request(draft, &request.name, &request.access_key_id, &destination)?;
    }

    let storage_id = match request.storage_id.clone() {
        Some(storage_id) => storage_id,
        None => StorageId::parse(
            generated_named_id("storage").map_err(|_| StorageSetupError::persistence())?,
        )
        .map_err(|_| StorageSetupError::persistence())?,
    };
    // Complete every fallible ID operation before copying the zeroizing
    // request secret into StorageConfig's ordinary String allocation. From
    // this point onward PendingStorage owns and zeroizes that copy on error.
    let secret_access_key = match request.secret_access_key.as_ref() {
        Some(secret) => secret.expose().to_string(),
        None => match (existing.as_ref(), draft.as_mut()) {
            (Some(storage), _) => storage.storage().secret_access_key.clone(),
            (None, Some(draft)) => draft.take_secret()?,
            (None, None) => return Err(secret_access_key_required()),
        },
    };
    let pending = PendingStorage::new(StorageConfig {
        id: storage_id.clone(),
        name: request.name,
        kind: StorageKind::S3,
        bucket: destination.bucket().to_string(),
        access_key_id: request.access_key_id,
        secret_access_key,
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
    ensure_upsert_destination_available(&before, &destination, request.storage_id.as_ref())?;

    Ok(PreparedR2Upsert {
        _operation: operation,
        destination,
        storage_id,
        target_storage_id: request.storage_id,
        expected_config_revision: request.expected_config_revision,
        existing,
        pending,
    })
}

/// A loaded setup draft whose ordinary `String` secret is zeroized on every
/// return path. When its secret is consumed, [`PendingStorage`] immediately
/// assumes ownership of the moved allocation and applies the same guarantee.
struct PinnedR2StorageSetupDraft(StorageSetupDraft);

impl PinnedR2StorageSetupDraft {
    fn storage(&self) -> &StorageConfig {
        &self.0.storage
    }

    fn take_secret(&mut self) -> Result<String, StorageSetupError> {
        let secret = std::mem::take(&mut self.0.storage.secret_access_key);
        if secret.is_empty() {
            Err(secret_access_key_required())
        } else {
            Ok(secret)
        }
    }
}

impl Drop for PinnedR2StorageSetupDraft {
    fn drop(&mut self) {
        self.0.storage.secret_access_key.zeroize();
    }
}

fn load_pinned_r2_storage_setup_draft(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
    draft_revision: u64,
    draft_storage_id: &StorageId,
) -> Result<PinnedR2StorageSetupDraft, StorageSetupError> {
    let draft = repository
        .load_storage_setup_draft(draft_id)
        .map_err(|_| StorageSetupError::persistence())?
        .ok_or_else(storage_setup_draft_not_found)?;
    let pinned = PinnedR2StorageSetupDraft(draft);
    if pinned.0.revision != draft_revision {
        return Err(storage_setup_draft_revision_conflict());
    }
    if &pinned.0.storage.id != draft_storage_id {
        return Err(storage_setup_draft_storage_conflict());
    }
    if pinned.0.storage.kind != StorageKind::S3 {
        return Err(storage_setup_draft_not_r2());
    }
    Ok(pinned)
}

fn validate_pinned_r2_request(
    draft: &PinnedR2StorageSetupDraft,
    name: &str,
    access_key_id: &str,
    destination: &R2Destination,
) -> Result<(), StorageSetupError> {
    if draft.storage().access_key_id != access_key_id {
        return Err(storage_setup_draft_access_key_conflict());
    }
    if draft.storage().name != name {
        return Err(storage_setup_draft_settings_conflict());
    }
    let drafted_destination = destination_from_storage(draft.storage())
        .map_err(|_| storage_setup_draft_settings_conflict())?;
    if &drafted_destination != destination {
        return Err(storage_setup_draft_settings_conflict());
    }
    Ok(())
}

/// Reconcile interrupted probes at the prepared destination, journal one
/// fresh probe, verify the pending storage remotely, and run `on_verified`
/// inside the same journal bracket. The prepared host guard stays alive
/// across reconciliation, probing, `on_verified`, and the journal finish.
fn probe_r2_upsert<T>(
    repository: &LocalStateRepository,
    prepared: PreparedR2Upsert,
    probe: &impl CapabilityProbe,
    on_verified: impl FnOnce(VerifiedR2Upsert) -> Result<T, StorageSetupError>,
) -> Result<T, StorageSetupError> {
    let PreparedR2Upsert {
        _operation,
        destination,
        storage_id,
        target_storage_id,
        expected_config_revision,
        existing,
        pending,
    } = prepared;
    let journal = R2SetupJournalStore::new(repository);
    let old_destination = existing
        .as_ref()
        .map(|storage| destination_from_storage(storage.storage()))
        .transpose()?;
    match (existing.as_ref(), old_destination.as_ref()) {
        (Some(storage), Some(old_destination)) if old_destination == &destination => {
            // Schema 1 journals identify a destination but intentionally do
            // not retain credentials. Try the stored credentials first, then
            // the pending replacement credentials only when the first cleanup
            // fails. This covers a probe interrupted during an earlier
            // same-destination credential replacement without persisting a
            // secret or adding a journal migration.
            reconcile_journaled_probes_with_fallback(
                &journal,
                probe,
                storage.storage(),
                pending.storage(),
                old_destination,
            )?;
        }
        (Some(storage), Some(old_destination)) => {
            // A destination change keeps the credential boundary exact: old
            // journals use old credentials and new journals use the pending
            // destination's credentials.
            reconcile_journaled_probes(&journal, probe, storage.storage(), old_destination)?;
            reconcile_journaled_probes(&journal, probe, pending.storage(), &destination)?;
        }
        _ => {
            reconcile_journaled_probes(&journal, probe, pending.storage(), &destination)?;
        }
    }

    let (setup_id, mut entry) = start_probe_journal(&journal, &destination)?;
    let result = probe
        .verify(pending.storage(), setup_id.as_str())
        .and_then(|()| {
            on_verified(VerifiedR2Upsert {
                storage_id,
                target_storage_id,
                destination,
                expected_config_revision,
                pending,
            })
        });
    finish_probe_journal(&journal, &mut entry, result)
}

fn upsert_r2_storage_with_probe(
    repository: &LocalStateRepository,
    request: UpsertR2StorageRequest,
    probe: &impl CapabilityProbe,
) -> Result<UpsertR2StorageResult, StorageSetupError> {
    let draft_id = request.draft_id.clone();
    let prepared = prepare_r2_upsert(repository, request)?;
    let result = probe_r2_upsert(repository, prepared, probe, |verified| {
        let VerifiedR2Upsert {
            storage_id,
            target_storage_id,
            destination,
            expected_config_revision,
            pending,
        } = verified;
        let expected_revision = expected_config_revision;
        let destination_for_commit = destination;
        let storage_id_for_commit = storage_id.clone();
        let (_, committed) = repository
            .transact_config(move |config| {
                if config.revision != expected_revision {
                    return Err(revision_conflict());
                }
                validate_r2_storage_destinations(config)?;
                ensure_upsert_destination_available(
                    config,
                    &destination_for_commit,
                    target_storage_id.as_ref(),
                )?;

                match target_storage_id.as_ref() {
                    Some(target_storage_id) => {
                        let index = config
                            .storages
                            .iter()
                            .position(|storage| &storage.id == target_storage_id)
                            .ok_or_else(storage_not_found)?;
                        if config.storages[index].kind != StorageKind::S3 {
                            return Err(storage_not_r2());
                        }
                        config.storages[index] = pending.into_storage();
                    }
                    None => {
                        if config
                            .storages
                            .iter()
                            .any(|storage| storage.id == storage_id_for_commit)
                        {
                            return Err(StorageSetupError::conflict(
                                "storage_id_conflict",
                                "Mallard could not allocate a unique storage ID.",
                            ));
                        }
                        config.storages.push(pending.into_storage());
                    }
                }
                validate_r2_storage_destinations(config)?;
                Ok(ConfigMutation::Changed(()))
            })
            .map_err(|error| match error {
                ConfigTransactionError::Mutation(error) => error,
                ConfigTransactionError::Persistence(_) => StorageSetupError::persistence(),
            })?;

        Ok(UpsertR2StorageResult {
            storage_id,
            config_revision: committed.revision,
            verified: true,
        })
    })?;
    // The draft served its purpose the moment the commit landed. Discard is
    // best-effort and never fails a committed upsert.
    if let Some(draft_id) = draft_id {
        let _ = repository.delete_storage_setup_draft(&draft_id);
    }
    Ok(result)
}

fn test_r2_storage_with_probe(
    repository: &LocalStateRepository,
    request: UpsertR2StorageRequest,
    probe: &impl CapabilityProbe,
) -> Result<TestR2StorageResult, StorageSetupError> {
    let prepared = prepare_r2_upsert(repository, request)?;
    let config_revision = prepared.expected_config_revision;
    probe_r2_upsert(repository, prepared, probe, |verified| {
        // Dropping the verified payload zeroizes the pending secret copy;
        // no config transaction exists on this path by construction.
        drop(verified);
        Ok(TestR2StorageResult {
            config_revision,
            verified: true,
        })
    })
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
        reconcile_journaled_probes(&journal, probe, &storage, &destination)?;
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
        created_at: timestamp,
        updated_at: timestamp,
        error_code: None,
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
) -> Result<(), StorageSetupError> {
    reconcile_journaled_probes_with_optional_fallback(journal, probe, storage, None, destination)
}

fn reconcile_journaled_probes_with_fallback(
    journal: &R2SetupJournalStore,
    probe: &impl CapabilityProbe,
    stored: &StorageConfig,
    pending: &StorageConfig,
    destination: &R2Destination,
) -> Result<(), StorageSetupError> {
    let fallback = if same_probe_credentials(stored, pending) {
        None
    } else {
        Some(pending)
    };
    reconcile_journaled_probes_with_optional_fallback(journal, probe, stored, fallback, destination)
}

fn reconcile_journaled_probes_with_optional_fallback(
    journal: &R2SetupJournalStore,
    probe: &impl CapabilityProbe,
    primary: &StorageConfig,
    fallback: Option<&StorageConfig>,
    destination: &R2Destination,
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
        let mut reconciled = probe.reconcile(primary, entry.setup_id.as_str()).is_ok();
        if !reconciled {
            if let Some(fallback) = fallback {
                reconciled = probe.reconcile(fallback, entry.setup_id.as_str()).is_ok();
            }
        }
        if !reconciled {
            entry.stage = R2SetupStage::CleanupRequired;
            entry.error_code = Some("probe_cleanup_failed".to_string());
            entry.updated_at = entry.updated_at.saturating_add(1);
            let _ = journal.save(&entry);
            return Err(StorageSetupError::cleanup_required(entry.setup_id.as_str()));
        }
        journal
            .remove(&entry.setup_id)
            .map_err(|_| StorageSetupError::persistence())?;
    }
    Ok(())
}

fn same_probe_credentials(first: &StorageConfig, second: &StorageConfig) -> bool {
    first.access_key_id == second.access_key_id
        && first.secret_access_key == second.secret_access_key
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

fn ensure_upsert_destination_available(
    config: &SyncConfig,
    destination: &R2Destination,
    excluded_storage_id: Option<&StorageId>,
) -> Result<(), StorageSetupError> {
    for storage in &config.storages {
        if storage.kind != StorageKind::S3
            || excluded_storage_id.is_some_and(|excluded| excluded == &storage.id)
        {
            continue;
        }
        if destination_from_storage(storage)? == *destination {
            return Err(duplicate_destination());
        }
    }
    Ok(())
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

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().max(1))
        .unwrap_or(1)
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

fn secret_access_key_required() -> StorageSetupError {
    StorageSetupError::invalid(
        "r2_secret_access_key_required",
        "The R2 Secret Access Key is required for new storage.",
    )
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

fn storage_setup_draft_storage_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "storage_setup_draft_storage_conflict",
        "The storage setup draft identity changed; review the latest settings in Mallard.",
    )
}

fn storage_setup_draft_not_r2() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_setup_draft_not_r2",
        "The requested storage setup draft is not an R2 draft.",
    )
}

fn storage_setup_draft_access_key_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "storage_setup_draft_access_key_conflict",
        "The R2 Access Key ID no longer matches the setup draft; review the latest settings in Mallard.",
    )
}

fn storage_setup_draft_settings_conflict() -> StorageSetupError {
    StorageSetupError::conflict(
        "storage_setup_draft_settings_conflict",
        "The R2 settings no longer match the saved setup draft; review the latest settings in Mallard.",
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
        "Mallard storage configuration changed; reload it before saving storage.",
    )
}

fn storage_not_found() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_not_found",
        "The requested Mallard storage does not exist.",
    )
}

fn storage_not_r2() -> StorageSetupError {
    StorageSetupError::invalid(
        "storage_not_r2",
        "The requested Mallard storage is not R2 storage.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

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

    struct RevisionBumpProbe {
        repository: LocalStateRepository,
        calls: Cell<usize>,
    }

    impl CapabilityProbe for RevisionBumpProbe {
        fn verify(
            &self,
            _storage: &StorageConfig,
            _setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.calls.set(self.calls.get() + 1);
            let config = self.repository.load_config().unwrap();
            self.repository.save_config(config).unwrap();
            Ok(())
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ProbeEvent {
        Reconcile {
            access_key_id: String,
            secret_access_key: String,
        },
        Verify {
            access_key_id: String,
            secret_access_key: String,
        },
    }

    struct RecordingProbe {
        events: RefCell<Vec<ProbeEvent>>,
        reconcile_access_key_id: Option<String>,
    }

    impl RecordingProbe {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                reconcile_access_key_id: None,
            }
        }

        fn requiring_reconcile_access_key(access_key_id: &str) -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                reconcile_access_key_id: Some(access_key_id.to_string()),
            }
        }
    }

    impl CapabilityProbe for RecordingProbe {
        fn verify(
            &self,
            storage: &StorageConfig,
            _setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.events.borrow_mut().push(ProbeEvent::Verify {
                access_key_id: storage.access_key_id.clone(),
                secret_access_key: storage.secret_access_key.clone(),
            });
            Ok(())
        }

        fn reconcile(
            &self,
            storage: &StorageConfig,
            _setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.events.borrow_mut().push(ProbeEvent::Reconcile {
                access_key_id: storage.access_key_id.clone(),
                secret_access_key: storage.secret_access_key.clone(),
            });
            match &self.reconcile_access_key_id {
                Some(required) if required != &storage.access_key_id => {
                    Err(StorageSetupError::remote())
                }
                _ => Ok(()),
            }
        }
    }

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path()).unwrap()
    }

    fn default_url(bucket: &str) -> String {
        format!("https://{ACCOUNT}.r2.cloudflarestorage.com/{bucket}")
    }

    fn upsert_request(
        storage_id: Option<StorageId>,
        name: &str,
        api_url: &str,
        access_key_id: &str,
        secret_access_key: Option<&str>,
        expected_config_revision: u64,
    ) -> UpsertR2StorageRequest {
        UpsertR2StorageRequest {
            storage_id,
            draft_id: None,
            draft_revision: None,
            draft_storage_id: None,
            name: name.to_string(),
            api_url: api_url.to_string(),
            access_key_id: access_key_id.to_string(),
            secret_access_key: secret_access_key
                .map(|secret| SecretString::new(secret.to_string())),
            expected_config_revision,
        }
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
    fn verified_upsert_creates_exact_schema_one_r2_storage() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();
        let url = default_url("team-backup");

        let result = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(None, "Team backup", &url, ACCESS, Some(SECRET), 0),
            &probe,
        )
        .unwrap();

        assert!(result.verified);
        assert_eq!(result.config_revision, 1);
        let config = repository.load_config().unwrap();
        assert_eq!(config.schema, 1);
        assert_eq!(config.revision, result.config_revision);
        assert_eq!(config.storages.len(), 1);
        assert_eq!(
            config.storages[0],
            StorageConfig {
                id: result.storage_id,
                name: "Team backup".to_string(),
                kind: StorageKind::S3,
                bucket: "team-backup".to_string(),
                access_key_id: ACCESS.to_string(),
                secret_access_key: SECRET.to_string(),
                account_id: ACCOUNT.to_string(),
                s3_endpoint: format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
                region: "auto".to_string(),
                local_dir: String::new(),
                supports_conditional_writes: Some(true),
            }
        );
        assert_eq!(probe.calls.get(), 1);
    }

    #[test]
    fn verified_upsert_preserves_existing_secret_when_edit_submits_none() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let create_probe = FakeProbe::successful();
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &create_probe,
        )
        .unwrap();
        let edit_probe = RecordingProbe::new();

        let edited = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id.clone()),
                "Renamed backup",
                &default_url("team-backup"),
                ACCESS,
                None,
                created.config_revision,
            ),
            &edit_probe,
        )
        .unwrap();

        assert_eq!(edited.storage_id, created.storage_id);
        assert_eq!(edited.config_revision, 2);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages.len(), 1);
        assert_eq!(config.storages[0].name, "Renamed backup");
        assert_eq!(config.storages[0].secret_access_key, SECRET);
        assert_eq!(
            edit_probe.events.borrow().as_slice(),
            [ProbeEvent::Verify {
                access_key_id: ACCESS.to_string(),
                secret_access_key: SECRET.to_string(),
            }]
        );
    }

    #[test]
    fn verified_upsert_replaces_existing_r2_in_place_with_the_same_id() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Old backup",
                &default_url("old-bucket"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &probe,
        )
        .unwrap();

        let replaced = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id.clone()),
                "New backup",
                &default_url("new-bucket"),
                "replacement-access",
                Some("replacement-secret"),
                created.config_revision,
            ),
            &probe,
        )
        .unwrap();

        assert_eq!(replaced.storage_id, created.storage_id);
        assert_eq!(replaced.config_revision, 2);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages.len(), 1);
        let storage = &config.storages[0];
        assert_eq!(storage.id, created.storage_id);
        assert_eq!(storage.name, "New backup");
        assert_eq!(storage.bucket, "new-bucket");
        assert_eq!(storage.access_key_id, "replacement-access");
        assert_eq!(storage.secret_access_key, "replacement-secret");
    }

    #[test]
    fn verified_upsert_rejects_stale_revision_before_remote_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let create_probe = FakeProbe::successful();
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &create_probe,
        )
        .unwrap();
        let before = repository.load_config().unwrap();
        let stale_probe = FakeProbe::successful();

        let error = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id),
                "Stale edit",
                &default_url("team-backup"),
                ACCESS,
                None,
                0,
            ),
            &stale_probe,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(error.code(), "config_revision_conflict");
        assert_eq!(stale_probe.calls.get(), 0);
        assert_eq!(stale_probe.reconcile_calls.get(), 0);
        assert_eq!(repository.load_config().unwrap(), before);
    }

    #[test]
    fn verified_upsert_rechecks_revision_inside_commit_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = RevisionBumpProbe {
            repository: repository.clone(),
            calls: Cell::new(0),
        };

        let error = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &probe,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(error.code(), "config_revision_conflict");
        assert_eq!(probe.calls.get(), 1);
        let config = repository.load_config().unwrap();
        assert_eq!(config.revision, 1);
        assert!(config.storages.is_empty());
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn verified_upsert_remote_failure_does_not_commit() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe {
            calls: Cell::new(0),
            outcome: RefCell::new(Some(StorageSetupError::remote())),
            reconcile_calls: Cell::new(0),
            reconcile_outcome: RefCell::new(None),
        };

        let error = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &probe,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::RemoteFailure);
        assert_eq!(probe.calls.get(), 1);
        let config = repository.load_config().unwrap();
        assert_eq!(config.revision, 0);
        assert!(config.storages.is_empty());
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn verified_upsert_rejects_destination_conflict_before_remote_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let create_probe = FakeProbe::successful();
        let first = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "First",
                &default_url("first-bucket"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &create_probe,
        )
        .unwrap();
        let second = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Second",
                &default_url("second-bucket"),
                "second-access",
                Some("second-secret"),
                first.config_revision,
            ),
            &create_probe,
        )
        .unwrap();
        let before = repository.load_config().unwrap();
        let conflict_probe = FakeProbe::successful();

        let error = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(first.storage_id),
                "Conflicting edit",
                &default_url("second-bucket"),
                ACCESS,
                None,
                second.config_revision,
            ),
            &conflict_probe,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(error.code(), "r2_destination_conflict");
        assert_eq!(conflict_probe.calls.get(), 0);
        assert_eq!(conflict_probe.reconcile_calls.get(), 0);
        assert_eq!(repository.load_config().unwrap(), before);
    }

    #[test]
    fn verified_upsert_reconciles_old_journal_with_old_credentials_before_replacement_probe() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let create_probe = FakeProbe::successful();
        let url = default_url("team-backup");
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(None, "Team backup", &url, ACCESS, Some(SECRET), 0),
            &create_probe,
        )
        .unwrap();
        let destination = parse_r2_api_url(&url).unwrap();
        let journal = R2SetupJournalStore::new(&repository);
        start_probe_journal(&journal, &destination).unwrap();
        let probe = RecordingProbe::new();

        upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id),
                "Team backup",
                &url,
                "replacement-access",
                Some("replacement-secret"),
                created.config_revision,
            ),
            &probe,
        )
        .unwrap();

        assert_eq!(
            probe.events.borrow().as_slice(),
            [
                ProbeEvent::Reconcile {
                    access_key_id: ACCESS.to_string(),
                    secret_access_key: SECRET.to_string(),
                },
                ProbeEvent::Verify {
                    access_key_id: "replacement-access".to_string(),
                    secret_access_key: "replacement-secret".to_string(),
                },
            ]
        );
        assert!(journal.list().unwrap().is_empty());
    }

    #[test]
    fn verified_upsert_recovers_interrupted_same_destination_replacement_with_pending_credentials()
    {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let create_probe = FakeProbe::successful();
        let url = default_url("team-backup");
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(None, "Team backup", &url, ACCESS, Some(SECRET), 0),
            &create_probe,
        )
        .unwrap();
        let journal = R2SetupJournalStore::new(&repository);
        let interrupted_probe = FakeProbe {
            calls: Cell::new(0),
            outcome: RefCell::new(Some(StorageSetupError::cleanup_required(
                "interrupted-replacement",
            ))),
            reconcile_calls: Cell::new(0),
            reconcile_outcome: RefCell::new(None),
        };
        let interrupted = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id.clone()),
                "Team backup",
                &url,
                "replacement-access",
                Some("replacement-secret"),
                created.config_revision,
            ),
            &interrupted_probe,
        )
        .unwrap_err();
        assert_eq!(interrupted.kind(), StorageSetupErrorKind::CleanupRequired);
        assert_eq!(journal.list().unwrap().len(), 1);
        let probe = RecordingProbe::requiring_reconcile_access_key("replacement-access");

        let replaced = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id.clone()),
                "Team backup",
                &url,
                "replacement-access",
                Some("replacement-secret"),
                created.config_revision,
            ),
            &probe,
        )
        .unwrap();

        assert_eq!(replaced.storage_id, created.storage_id);
        assert_eq!(replaced.config_revision, 2);
        assert_eq!(
            probe.events.borrow().as_slice(),
            [
                ProbeEvent::Reconcile {
                    access_key_id: ACCESS.to_string(),
                    secret_access_key: SECRET.to_string(),
                },
                ProbeEvent::Reconcile {
                    access_key_id: "replacement-access".to_string(),
                    secret_access_key: "replacement-secret".to_string(),
                },
                ProbeEvent::Verify {
                    access_key_id: "replacement-access".to_string(),
                    secret_access_key: "replacement-secret".to_string(),
                },
            ]
        );
        assert!(journal.list().unwrap().is_empty());
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages[0].access_key_id, "replacement-access");
        assert_eq!(config.storages[0].secret_access_key, "replacement-secret");
    }

    #[test]
    fn verified_upsert_keeps_cleanup_journal_when_both_same_destination_credentials_fail() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let create_probe = FakeProbe::successful();
        let url = default_url("team-backup");
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(None, "Team backup", &url, ACCESS, Some(SECRET), 0),
            &create_probe,
        )
        .unwrap();
        let before = repository.load_config().unwrap();
        let journal = R2SetupJournalStore::new(&repository);
        let interrupted_probe = FakeProbe {
            calls: Cell::new(0),
            outcome: RefCell::new(Some(StorageSetupError::cleanup_required(
                "interrupted-replacement",
            ))),
            reconcile_calls: Cell::new(0),
            reconcile_outcome: RefCell::new(None),
        };
        let interrupted = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id.clone()),
                "Team backup",
                &url,
                "replacement-access",
                Some("replacement-secret"),
                created.config_revision,
            ),
            &interrupted_probe,
        )
        .unwrap_err();
        let interrupted_setup_id = interrupted.setup_id().unwrap().to_string();
        assert_eq!(interrupted.kind(), StorageSetupErrorKind::CleanupRequired);
        let probe = RecordingProbe::requiring_reconcile_access_key("unavailable-access");

        let error = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(created.storage_id),
                "Team backup",
                &url,
                "replacement-access",
                Some("replacement-secret"),
                created.config_revision,
            ),
            &probe,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::CleanupRequired);
        assert_eq!(error.setup_id(), Some(interrupted_setup_id.as_str()));
        assert_eq!(
            probe.events.borrow().as_slice(),
            [
                ProbeEvent::Reconcile {
                    access_key_id: ACCESS.to_string(),
                    secret_access_key: SECRET.to_string(),
                },
                ProbeEvent::Reconcile {
                    access_key_id: "replacement-access".to_string(),
                    secret_access_key: "replacement-secret".to_string(),
                },
            ]
        );
        assert_eq!(repository.load_config().unwrap(), before);
        let entries = journal.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].stage, R2SetupStage::CleanupRequired);
        assert_eq!(
            entries[0].error_code.as_deref(),
            Some("probe_cleanup_failed")
        );
    }

    #[test]
    fn verify_runs_the_probe_without_mutating_config() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let connect_probe = FakeProbe::successful();
        let created = upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &connect_probe,
        )
        .unwrap();
        let connected = repository.load_config().unwrap();
        let storage_id = created.storage_id;
        let verify_probe = FakeProbe::successful();

        let verified = verify_storage_with_probe(&repository, &storage_id, &verify_probe).unwrap();
        assert_eq!(verified.storage_id, storage_id);
        assert_eq!(verified.config_revision, connected.revision);
        assert!(verified.verified);
        assert_eq!(verify_probe.calls.get(), 1);
        assert_eq!(repository.load_config().unwrap().revision, 1);
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

    #[test]
    fn draft_operation_validates_the_exact_target_before_cloudflare_work() {
        let first_temp = tempfile::tempdir().unwrap();
        let first_repository = repository(&first_temp);
        let draft = stored_draft(&first_repository, "old-secret");
        let held = first_repository.acquire_host_operation().unwrap();

        let invalid = begin_r2_storage_draft_operation(
            &first_repository,
            &draft.draft_id,
            draft.revision,
            " bad ",
        )
        .err()
        .expect("invalid name must fail");
        assert_eq!(invalid.kind(), StorageSetupErrorKind::InvalidInput);
        assert_eq!(invalid.code(), "storage_name_invalid");
        let busy = begin_r2_storage_draft_operation(
            &first_repository,
            &draft.draft_id,
            draft.revision,
            "Team backup",
        )
        .err()
        .expect("held operation must conflict");
        assert_eq!(busy.code(), "r2_setup_busy");
        drop(held);

        let stale = begin_r2_storage_draft_operation(
            &first_repository,
            &draft.draft_id,
            draft.revision + 1,
            "Team backup",
        )
        .err()
        .expect("stale revision must fail");
        assert_eq!(stale.code(), "storage_setup_draft_revision_conflict");

        let operation = begin_r2_storage_draft_operation(
            &first_repository,
            &draft.draft_id,
            draft.revision,
            "Team backup",
        )
        .unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let second_repository = repository(&second_temp);
        let error = prepare_r2_storage_draft_in_operation(
            &operation,
            &second_repository,
            PrepareR2StorageDraftRequest {
                api_url: default_url("team-backup"),
                access_key_id: ACCESS.to_string(),
                secret_access_key: SecretString::new(SECRET.to_string()),
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), StorageSetupErrorKind::PersistenceFailure);
        assert!(second_repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn browser_credentials_prepare_the_exact_draft_without_mutating_config() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, "old-secret");
        assert!(!repository.config_path().exists());
        let config_before = repository.load_config().unwrap();
        let operation = begin_r2_storage_draft_operation(
            &repository,
            &draft.draft_id,
            draft.revision,
            "Team backup",
        )
        .unwrap();

        let result = prepare_r2_storage_draft_in_operation(
            &operation,
            &repository,
            PrepareR2StorageDraftRequest {
                api_url: default_url("prepared-bucket"),
                access_key_id: "prepared-access".to_string(),
                secret_access_key: SecretString::new("prepared-secret".to_string()),
            },
        )
        .unwrap();

        assert_eq!(result.draft_id, draft.draft_id);
        assert_eq!(result.draft_revision, draft.revision + 1);
        let prepared = repository
            .load_storage_setup_draft(&result.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.storage.id, draft.storage.id);
        assert_eq!(prepared.storage.name, "Team backup");
        assert_eq!(prepared.storage.kind, StorageKind::S3);
        assert_eq!(prepared.storage.bucket, "prepared-bucket");
        assert_eq!(prepared.storage.access_key_id, "prepared-access");
        assert_eq!(prepared.storage.secret_access_key, "prepared-secret");
        assert_eq!(prepared.storage.account_id, ACCOUNT);
        assert_eq!(
            prepared.storage.s3_endpoint,
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com")
        );
        assert_eq!(prepared.storage.region, "auto");
        assert!(prepared.storage.local_dir.is_empty());
        assert_eq!(prepared.storage.supports_conditional_writes, None);
        assert_eq!(repository.load_config().unwrap(), config_before);
        assert!(!repository.config_path().exists());
    }

    #[test]
    fn copied_trimmed_name_targets_and_normalizes_the_same_editable_draft() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let mut draft = stored_draft(&repository, "old-secret");
        draft.storage.name = "  Team backup  ".to_string();
        let draft = repository.save_storage_setup_draft(draft).unwrap();
        let operation = begin_r2_storage_draft_operation(
            &repository,
            &draft.draft_id,
            draft.revision,
            "Team backup",
        )
        .unwrap();

        prepare_r2_storage_draft_in_operation(
            &operation,
            &repository,
            PrepareR2StorageDraftRequest {
                api_url: default_url("prepared-bucket"),
                access_key_id: "prepared-access".to_string(),
                secret_access_key: SecretString::new("prepared-secret".to_string()),
            },
        )
        .unwrap();

        let prepared = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.storage.name, "Team backup");
    }

    #[test]
    fn concurrent_form_edit_wins_the_exact_draft_cas() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, "old-secret");
        assert!(!repository.config_path().exists());
        let config_before = repository.load_config().unwrap();
        let operation = begin_r2_storage_draft_operation(
            &repository,
            &draft.draft_id,
            draft.revision,
            "Team backup",
        )
        .unwrap();

        let mut edited = draft.clone();
        edited.storage.bucket = "user-edit".to_string();
        edited.storage.secret_access_key = "user-secret".to_string();
        let edited = repository.save_storage_setup_draft(edited).unwrap();
        let error = prepare_r2_storage_draft_in_operation(
            &operation,
            &repository,
            PrepareR2StorageDraftRequest {
                api_url: default_url("prepared-bucket"),
                access_key_id: "prepared-access".to_string(),
                secret_access_key: SecretString::new("prepared-secret".to_string()),
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::Conflict);
        assert_eq!(error.code(), "storage_setup_draft_revision_conflict");
        let current = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(current, edited);
        assert_eq!(repository.load_config().unwrap(), config_before);
        assert!(!repository.config_path().exists());
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
    fn invalid_existing_s3_entry_blocks_verify_before_remote_work() {
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

        let verify_error = verify_storage_with_probe(&repository, &invalid_id, &probe).unwrap_err();
        assert_eq!(verify_error.code(), "r2_storage_destination_invalid");
        assert_eq!(probe.calls.get(), 0);
    }

    const DRAFT_SECRET: &str = "draft-secret-canary";

    fn stored_draft(
        repository: &LocalStateRepository,
        secret: &str,
    ) -> mallard_domain::StorageSetupDraft {
        let draft = mallard_domain::StorageSetupDraft {
            schema: mallard_domain::STORAGE_SETUP_DRAFT_SCHEMA,
            draft_id: StorageSetupDraftId::parse(generated_named_id("storagedraft").unwrap())
                .unwrap(),
            storage: StorageConfig {
                id: StorageId::parse(generated_named_id("storage").unwrap()).unwrap(),
                name: "Team backup".to_string(),
                kind: StorageKind::S3,
                bucket: "team-backup".to_string(),
                access_key_id: ACCESS.to_string(),
                secret_access_key: secret.to_string(),
                account_id: ACCOUNT.to_string(),
                s3_endpoint: format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
                region: "auto".to_string(),
                local_dir: String::new(),
                supports_conditional_writes: None,
            },
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };
        repository.save_storage_setup_draft(draft).unwrap()
    }

    fn draft_request(
        draft: Option<&mallard_domain::StorageSetupDraft>,
        secret: Option<&str>,
    ) -> UpsertR2StorageRequest {
        let mut request = upsert_request(
            None,
            "Team backup",
            &default_url("team-backup"),
            ACCESS,
            secret,
            0,
        );
        if let Some(draft) = draft {
            request.draft_id = Some(draft.draft_id.clone());
            request.draft_revision = Some(draft.revision);
            request.draft_storage_id = Some(draft.storage.id.clone());
        }
        request
    }

    #[test]
    fn verified_test_probes_without_committing_config_or_journal_residue() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let probe = FakeProbe::successful();

        let tested = test_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &probe,
        )
        .unwrap();

        assert!(tested.verified);
        assert_eq!(tested.config_revision, 0);
        assert_eq!(probe.calls.get(), 1);
        let config = repository.load_config().unwrap();
        assert_eq!(config.revision, 0);
        assert!(config.storages.is_empty());
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_uses_the_stored_secret_for_an_existing_storage() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let connect_probe = FakeProbe::successful();
        upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &connect_probe,
        )
        .unwrap();
        let connected = repository.load_config().unwrap();
        let probe = RecordingProbe::new();

        test_r2_storage_with_probe(
            &repository,
            upsert_request(
                Some(connected.storages[0].id.clone()),
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                None,
                connected.revision,
            ),
            &probe,
        )
        .unwrap();

        assert_eq!(
            *probe.events.borrow(),
            vec![ProbeEvent::Verify {
                access_key_id: ACCESS.to_string(),
                secret_access_key: SECRET.to_string(),
            }]
        );
        assert_eq!(repository.load_config().unwrap(), connected);
    }

    #[test]
    fn test_resolves_the_draft_secret_and_keeps_the_draft() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let probe = RecordingProbe::new();

        test_r2_storage_with_probe(&repository, draft_request(Some(&draft), None), &probe).unwrap();

        assert_eq!(
            *probe.events.borrow(),
            vec![ProbeEvent::Verify {
                access_key_id: ACCESS.to_string(),
                secret_access_key: DRAFT_SECRET.to_string(),
            }]
        );
        // A test never consumes the draft — only a committed upsert does.
        assert!(repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .is_some());
        assert!(repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn draft_secret_resolution_requires_the_complete_exact_reviewed_reference() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);

        let mut incomplete = draft_request(Some(&draft), None);
        incomplete.draft_revision = None;
        let probe = FakeProbe::successful();
        let error = test_r2_storage_with_probe(&repository, incomplete, &probe).unwrap_err();
        assert_eq!(error.code(), "storage_draft_request_invalid");
        assert_eq!(probe.calls.get(), 0);

        let mut stale = draft_request(Some(&draft), None);
        stale.draft_revision = Some(draft.revision + 1);
        let probe = FakeProbe::successful();
        let error = test_r2_storage_with_probe(&repository, stale, &probe).unwrap_err();
        assert_eq!(error.code(), "storage_setup_draft_revision_conflict");
        assert_eq!(probe.calls.get(), 0);

        let mut wrong_storage = draft_request(Some(&draft), None);
        wrong_storage.draft_storage_id =
            Some(StorageId::parse(generated_named_id("storage").unwrap()).unwrap());
        let probe = FakeProbe::successful();
        let error = test_r2_storage_with_probe(&repository, wrong_storage, &probe).unwrap_err();
        assert_eq!(error.code(), "storage_setup_draft_storage_conflict");
        assert_eq!(probe.calls.get(), 0);

        let mut wrong_access_key = draft_request(Some(&draft), None);
        wrong_access_key.access_key_id = "different-reviewed-access-key".to_string();
        let probe = FakeProbe::successful();
        let error = test_r2_storage_with_probe(&repository, wrong_access_key, &probe).unwrap_err();
        assert_eq!(error.code(), "storage_setup_draft_access_key_conflict");
        assert_eq!(probe.calls.get(), 0);

        let mut wrong_name = draft_request(Some(&draft), None);
        wrong_name.name = "Different reviewed name".to_string();
        let probe = FakeProbe::successful();
        let error = test_r2_storage_with_probe(&repository, wrong_name, &probe).unwrap_err();
        assert_eq!(error.code(), "storage_setup_draft_settings_conflict");
        assert_eq!(probe.calls.get(), 0);

        let mut wrong_destination = draft_request(Some(&draft), None);
        wrong_destination.api_url = default_url("different-bucket");
        let probe = FakeProbe::successful();
        let error = test_r2_storage_with_probe(&repository, wrong_destination, &probe).unwrap_err();
        assert_eq!(error.code(), "storage_setup_draft_settings_conflict");
        assert_eq!(probe.calls.get(), 0);
    }

    #[test]
    fn draft_secret_resolution_rejects_a_non_r2_draft() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let mut draft = stored_draft(&repository, DRAFT_SECRET);
        draft.storage.kind = StorageKind::Local;
        let draft = repository.save_storage_setup_draft(draft).unwrap();
        let probe = FakeProbe::successful();

        let error =
            test_r2_storage_with_probe(&repository, draft_request(Some(&draft), None), &probe)
                .unwrap_err();

        assert_eq!(error.code(), "storage_setup_draft_not_r2");
        assert_eq!(probe.calls.get(), 0);
    }

    #[test]
    fn a_pinned_draft_access_key_mismatch_is_rejected_even_with_an_explicit_secret() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let probe = FakeProbe::successful();
        let mut request = draft_request(Some(&draft), Some(SECRET));
        request.access_key_id = "manually-entered-access-key".to_string();

        let error = test_r2_storage_with_probe(&repository, request, &probe).unwrap_err();

        assert_eq!(error.code(), "storage_setup_draft_access_key_conflict");
        assert_eq!(probe.calls.get(), 0);
    }

    #[test]
    fn test_rejects_stale_revision_and_destination_conflicts_before_remote_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let connect_probe = FakeProbe::successful();
        upsert_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &connect_probe,
        )
        .unwrap();
        let revision = repository.load_config().unwrap().revision;

        let stale_probe = FakeProbe::successful();
        let stale = test_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Another name",
                &default_url("another-bucket"),
                ACCESS,
                Some(SECRET),
                revision + 7,
            ),
            &stale_probe,
        )
        .unwrap_err();
        assert_eq!(stale.code(), "config_revision_conflict");
        assert_eq!(stale_probe.calls.get(), 0);
        assert_eq!(stale_probe.reconcile_calls.get(), 0);

        let conflict_probe = FakeProbe::successful();
        let conflict = test_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Different name",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                revision,
            ),
            &conflict_probe,
        )
        .unwrap_err();
        assert_eq!(conflict.code(), "r2_destination_conflict");
        assert_eq!(conflict_probe.calls.get(), 0);
        assert_eq!(conflict_probe.reconcile_calls.get(), 0);
    }

    #[test]
    fn test_takes_the_exclusive_setup_operation() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let held = repository.acquire_host_operation().unwrap();
        let probe = FakeProbe::successful();

        let error = test_r2_storage_with_probe(
            &repository,
            upsert_request(
                None,
                "Team backup",
                &default_url("team-backup"),
                ACCESS,
                Some(SECRET),
                0,
            ),
            &probe,
        )
        .unwrap_err();

        assert_eq!(error.code(), "r2_setup_busy");
        assert_eq!(probe.calls.get(), 0);
        drop(held);
    }

    #[test]
    fn test_keeps_the_cleanup_journal_and_failures_never_expose_canaries() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let probe = FakeProbe::successful();
        *probe.outcome.borrow_mut() = Some(StorageSetupError::cleanup_required(
            "r2setup-111111111111111111111111",
        ));

        let error =
            test_r2_storage_with_probe(&repository, draft_request(Some(&draft), None), &probe)
                .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::CleanupRequired);
        assert_eq!(
            R2SetupJournalStore::new(&repository).list().unwrap().len(),
            1
        );
        let output = format!("{error:?} {error}");
        for canary in [ACCOUNT, "team-backup", ACCESS, SECRET, DRAFT_SECRET] {
            assert!(!output.contains(canary), "leaked {canary}");
        }
        assert!(repository.load_config().unwrap().storages.is_empty());

        // The interrupted test's journal entry is reconciled by the next
        // probing operation, exactly like an interrupted save.
        *probe.outcome.borrow_mut() = None;
        test_r2_storage_with_probe(&repository, draft_request(Some(&draft), None), &probe).unwrap();
        assert_eq!(probe.reconcile_calls.get(), 1);
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn upsert_resolves_the_draft_secret_and_discards_the_draft_on_success() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let probe = RecordingProbe::new();

        let created =
            upsert_r2_storage_with_probe(&repository, draft_request(Some(&draft), None), &probe)
                .unwrap();

        assert!(created.verified);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages[0].secret_access_key, DRAFT_SECRET);
        assert!(repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn upsert_keeps_the_draft_when_the_probe_fails() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let probe = FakeProbe {
            calls: Cell::new(0),
            outcome: RefCell::new(Some(StorageSetupError::remote())),
            reconcile_calls: Cell::new(0),
            reconcile_outcome: RefCell::new(None),
        };

        upsert_r2_storage_with_probe(&repository, draft_request(Some(&draft), None), &probe)
            .unwrap_err();

        let kept = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(kept.storage.secret_access_key, DRAFT_SECRET);
        assert!(repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn explicit_request_secret_overrides_the_draft_secret_and_still_discards() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let probe = RecordingProbe::new();

        upsert_r2_storage_with_probe(
            &repository,
            draft_request(Some(&draft), Some(SECRET)),
            &probe,
        )
        .unwrap();

        assert_eq!(
            *probe.events.borrow(),
            vec![ProbeEvent::Verify {
                access_key_id: ACCESS.to_string(),
                secret_access_key: SECRET.to_string(),
            }]
        );
        assert_eq!(
            repository.load_config().unwrap().storages[0].secret_access_key,
            SECRET
        );
        assert!(repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_request_with_storage_id_and_draft_id_is_rejected_before_any_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        for run_test in [true, false] {
            let probe = FakeProbe::successful();
            let mut request = draft_request(Some(&draft), Some(SECRET));
            request.storage_id =
                Some(StorageId::parse(generated_named_id("storage").unwrap()).unwrap());
            let error = if run_test {
                test_r2_storage_with_probe(&repository, request, &probe).unwrap_err()
            } else {
                upsert_r2_storage_with_probe(&repository, request, &probe).unwrap_err()
            };
            assert_eq!(error.code(), "storage_draft_request_invalid");
            assert_eq!(probe.calls.get(), 0);
            assert_eq!(probe.reconcile_calls.get(), 0);
        }
    }

    #[test]
    fn a_missing_or_secretless_draft_fails_closed_without_remote_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);

        let probe = FakeProbe::successful();
        let unknown =
            StorageSetupDraftId::parse(generated_named_id("storagedraft").unwrap()).unwrap();
        let mut missing_request = draft_request(None, None);
        missing_request.draft_id = Some(unknown);
        missing_request.draft_revision = Some(1);
        missing_request.draft_storage_id =
            Some(StorageId::parse(generated_named_id("storage").unwrap()).unwrap());
        let missing = test_r2_storage_with_probe(&repository, missing_request, &probe).unwrap_err();
        assert_eq!(missing.code(), "storage_setup_draft_not_found");
        assert_eq!(probe.calls.get(), 0);

        let secretless = stored_draft(&repository, "");
        let required =
            test_r2_storage_with_probe(&repository, draft_request(Some(&secretless), None), &probe)
                .unwrap_err();
        assert_eq!(required.code(), "r2_secret_access_key_required");
        assert_eq!(probe.calls.get(), 0);
    }

    #[test]
    fn a_failed_draft_discard_never_fails_a_committed_upsert() {
        struct DraftDiscardSabotageProbe {
            path: PathBuf,
        }

        impl CapabilityProbe for DraftDiscardSabotageProbe {
            fn verify(
                &self,
                _storage: &StorageConfig,
                _setup_id: &str,
            ) -> Result<(), StorageSetupError> {
                std::fs::remove_file(&self.path).unwrap();
                std::fs::create_dir(&self.path).unwrap();
                Ok(())
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let draft = stored_draft(&repository, DRAFT_SECRET);
        let path = repository.storage_setup_draft_path(&draft.draft_id);
        // Sabotage the path only after preparation has validated the exact
        // readable draft, so the post-commit best-effort discard is the sole
        // operation that fails.
        let probe = DraftDiscardSabotageProbe { path };

        let created = upsert_r2_storage_with_probe(
            &repository,
            draft_request(Some(&draft), Some(SECRET)),
            &probe,
        )
        .unwrap();

        assert!(created.verified);
        assert_eq!(repository.load_config().unwrap().storages.len(), 1);
    }
}
