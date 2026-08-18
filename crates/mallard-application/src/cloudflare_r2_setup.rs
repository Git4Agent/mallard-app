//! Cloudflare-managed R2 provisioning.
//!
//! The bootstrap token is used only for Cloudflare's account-owned token
//! endpoints. Bucket inspection and creation always use a short-lived,
//! account-scoped R2 token; the durable token is scoped to one exact bucket.

use crate::storage_setup::{
    acquire_r2_setup_operation, connect_r2_storage_with_probe_for_setup, parse_r2_api_url,
    validate_r2_storage_destinations, CapabilityProbe, ConnectR2StorageRequest,
    ProductionCapabilityProbe, R2Jurisdiction, SecretString, StorageSetupError,
    StorageSetupErrorKind,
};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use mallard_domain::{generated_named_id, StorageConfig, StorageId, StorageKind};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::r2_setup_journal::{
    R2SetupId, R2SetupJournalEntry, R2SetupJournalStore, R2SetupJurisdiction, R2SetupStage,
    R2_SETUP_JOURNAL_SCHEMA,
};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::redirect::Policy as RedirectPolicy;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;
use tokio::runtime::Handle;
use zeroize::Zeroize;

const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
const MAX_CLOUDFLARE_RESPONSE_BYTES: u64 = 256 * 1024;
const MAX_BOOTSTRAP_TOKEN_BYTES: usize = 4 * 1024;
const TEMPORARY_TOKEN_TTL_MINUTES: i64 = 15;
const ACCOUNT_R2_WRITE: &str = "Workers R2 Storage Write";
const BUCKET_ITEM_WRITE: &str = "Workers R2 Storage Bucket Item Write";
const ACCOUNT_SCOPE: &str = "com.cloudflare.api.account";
const BUCKET_SCOPE: &str = "com.cloudflare.edge.r2.bucket";

/// Secret-bearing provisioning request. It deliberately has no `Debug`,
/// `Clone`, or `Serialize` implementation.
#[derive(Deserialize)]
pub struct ProvisionR2StorageRequest {
    pub name: String,
    pub account_id: String,
    pub bucket: String,
    pub jurisdiction: R2Jurisdiction,
    pub bootstrap_token: SecretString,
    #[serde(default)]
    pub expected_config_revision: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProvisionR2StorageResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub verified: bool,
    pub bucket_created: bool,
}

/// Secret-bearing explicit cleanup request. It deliberately has no `Debug`,
/// `Clone`, or `Serialize` implementation.
#[derive(Deserialize)]
pub struct CleanupR2SetupRequest {
    pub setup_id: String,
    pub bootstrap_token: SecretString,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CleanupR2SetupResult {
    pub setup_id: String,
    pub cleaned: bool,
}

pub type ProvisionR2StorageError = StorageSetupError;

pub fn provision_r2_storage(
    repository: &LocalStateRepository,
    runtime: Handle,
    request: ProvisionR2StorageRequest,
) -> Result<ProvisionR2StorageResult, ProvisionR2StorageError> {
    let _operation = acquire_r2_setup_operation(repository)?;
    let api = CloudflareHttpClient::new()?;
    let probe = ProductionCapabilityProbe { runtime };
    provision_r2_storage_with(repository, request, &api, &probe, Utc::now())
}

pub fn cleanup_r2_setup(
    repository: &LocalStateRepository,
    runtime: Handle,
    request: CleanupR2SetupRequest,
) -> Result<CleanupR2SetupResult, ProvisionR2StorageError> {
    let _operation = acquire_r2_setup_operation(repository)?;
    let api = CloudflareHttpClient::new()?;
    let probe = ProductionCapabilityProbe { runtime };
    cleanup_r2_setup_with(repository, request, &api, &probe, Utc::now())
}

fn provision_r2_storage_with(
    repository: &LocalStateRepository,
    request: ProvisionR2StorageRequest,
    api: &impl CloudflareR2Api,
    probe: &impl CapabilityProbe,
    now: DateTime<Utc>,
) -> Result<ProvisionR2StorageResult, StorageSetupError> {
    let account_id = canonical_account_id(&request.account_id)?;
    validate_storage_name(&request.name)?;
    validate_bootstrap_token(&request.bootstrap_token)?;
    let destination = destination_for(&account_id, request.jurisdiction, &request.bucket)?;
    let preflight = inspect_destination_preflight(
        repository,
        &destination,
        &request.name,
        request.expected_config_revision,
    )?;

    let journal = R2SetupJournalStore::new(repository);
    reconcile_previous_tokens(
        repository,
        &journal,
        api,
        &account_id,
        request.jurisdiction,
        &request.bucket,
        &request.bootstrap_token,
        now,
    )?;

    if let DestinationPreflight::Connected { storage, revision } = preflight {
        return verify_idempotent_provision(
            repository,
            &journal,
            probe,
            storage,
            revision,
            &account_id,
            request.jurisdiction,
            &request.bucket,
            now,
        );
    }

    let setup_id = R2SetupId::parse(
        generated_named_id("r2setup").map_err(|_| StorageSetupError::persistence())?,
    )
    .map_err(|_| StorageSetupError::persistence())?;
    let timestamp = nonzero_timestamp(now);
    let mut entry = R2SetupJournalEntry {
        schema: R2_SETUP_JOURNAL_SCHEMA,
        setup_id: setup_id.clone(),
        account_id: account_id.clone(),
        bucket: request.bucket.clone(),
        jurisdiction: journal_jurisdiction(request.jurisdiction),
        stage: R2SetupStage::Started,
        temporary_token_id: None,
        final_token_id: None,
        cleanup_token_id: None,
        bucket_created: false,
        created_at: timestamp,
        updated_at: timestamp,
        error_code: None,
        probe_cleanup_needed: false,
    };
    save_journal(&journal, &entry)?;

    let account_permission = match api.resolve_permission_group(
        &account_id,
        &request.bootstrap_token,
        ACCOUNT_R2_WRITE,
        ACCOUNT_SCOPE,
    ) {
        Ok(permission) => permission,
        Err(error) => {
            let _ = journal.remove(&setup_id);
            return Err(error);
        }
    };
    let bucket_permission = match api.resolve_permission_group(
        &account_id,
        &request.bootstrap_token,
        BUCKET_ITEM_WRITE,
        BUCKET_SCOPE,
    ) {
        Ok(permission) => permission,
        Err(error) => {
            let _ = journal.remove(&setup_id);
            return Err(error);
        }
    };

    let temporary = match api.create_token(
        &account_id,
        &request.bootstrap_token,
        &temporary_token_payload(setup_id.as_str(), &account_id, account_permission, now),
    ) {
        Ok(token) => token,
        Err(error) => {
            let _ = journal.remove(&setup_id);
            return Err(error);
        }
    };
    entry.temporary_token_id = Some(temporary.id.clone());
    advance_journal(&mut entry, R2SetupStage::TemporaryTokenCreated);
    if save_journal(&journal, &entry).is_err() {
        return revoke_after_unjournaled_creation(
            api,
            &account_id,
            &request.bootstrap_token,
            &temporary.id,
            &mut entry,
            &journal,
            false,
        );
    }

    let bucket_operation = match api.get_bucket(
        &account_id,
        &temporary.value,
        &request.bucket,
        request.jurisdiction,
    ) {
        Ok(BucketStatus::Present) => Ok(false),
        Ok(BucketStatus::Missing) => match api.create_bucket(
            &account_id,
            &temporary.value,
            &request.bucket,
            request.jurisdiction,
        ) {
            Ok(BucketCreateStatus::Created) => Ok(true),
            Ok(BucketCreateStatus::Conflict) => api
                .get_bucket(
                    &account_id,
                    &temporary.value,
                    &request.bucket,
                    request.jurisdiction,
                )
                .and_then(|status| match status {
                    BucketStatus::Present => Ok(false),
                    BucketStatus::Missing => Err(cloudflare_remote()),
                }),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    // The temporary token is revoked immediately regardless of bucket result.
    let temporary_id = temporary.id.clone();
    let temporary_revocation =
        api.revoke_token(&account_id, &request.bootstrap_token, &temporary_id);
    drop(temporary);
    if temporary_revocation.is_err() {
        mark_cleanup_required(&journal, &mut entry, "temporary_token_revoke_failed", false);
        return Err(cleanup_required(setup_id.as_str()));
    }
    entry.temporary_token_id = None;
    let bucket_created = match bucket_operation {
        Ok(created) => created,
        Err(error) => {
            let _ = journal.remove(&setup_id);
            return Err(error);
        }
    };
    entry.bucket_created = bucket_created;
    advance_journal(&mut entry, R2SetupStage::BucketReady);
    if let Err(error) = save_journal(&journal, &entry) {
        let _ = journal.remove(&setup_id);
        return Err(error);
    }

    let final_token = match api.create_token(
        &account_id,
        &request.bootstrap_token,
        &final_token_payload(
            setup_id.as_str(),
            &account_id,
            &request.bucket,
            request.jurisdiction,
            bucket_permission,
        ),
    ) {
        Ok(token) => token,
        Err(error) => {
            let _ = journal.remove(&setup_id);
            return Err(error);
        }
    };
    entry.final_token_id = Some(final_token.id.clone());
    advance_journal(&mut entry, R2SetupStage::FinalTokenCreated);
    if save_journal(&journal, &entry).is_err() {
        return revoke_after_unjournaled_creation(
            api,
            &account_id,
            &request.bootstrap_token,
            &final_token.id,
            &mut entry,
            &journal,
            false,
        );
    }

    let derived_secret = derive_s3_secret(&final_token.value);
    drop(final_token.value);
    let mut reconciliation_storage = StorageConfig {
        id: StorageId::parse("storage-r2-reconciliation").map_err(|_| cloudflare_remote())?,
        name: "R2 setup reconciliation".to_string(),
        kind: StorageKind::S3,
        bucket: request.bucket.clone(),
        access_key_id: final_token.id.clone(),
        secret_access_key: derived_secret.expose().to_string(),
        account_id: account_id.clone(),
        s3_endpoint: destination.endpoint().to_string(),
        region: "auto".to_string(),
        local_dir: String::new(),
        supports_conditional_writes: Some(true),
    };
    if let Err(error) = reconcile_previous_probes(
        &journal,
        probe,
        &reconciliation_storage,
        &account_id,
        request.jurisdiction,
        &request.bucket,
        &setup_id,
    ) {
        reconciliation_storage.secret_access_key.zeroize();
        return fail_after_final_token(
            api,
            &journal,
            &mut entry,
            &request.bootstrap_token,
            error,
            false,
        );
    }
    reconciliation_storage.secret_access_key.zeroize();

    advance_journal(&mut entry, R2SetupStage::ProbeRunning);
    if let Err(error) = save_journal(&journal, &entry) {
        return fail_after_final_token(
            api,
            &journal,
            &mut entry,
            &request.bootstrap_token,
            error,
            false,
        );
    }
    let connect_result = connect_r2_storage_with_probe_for_setup(
        repository,
        ConnectR2StorageRequest {
            name: request.name,
            api_url: destination.api_url().to_string(),
            access_key_id: final_token.id.clone(),
            secret_access_key: SecretString::new(derived_secret.expose().to_string()),
            expected_config_revision: request.expected_config_revision,
        },
        probe,
        setup_id.as_str().to_string(),
    );
    drop(derived_secret);

    let connected = match connect_result {
        Ok(result) => result,
        Err(error) => {
            let probe_cleanup_needed = error.kind() == StorageSetupErrorKind::CleanupRequired;
            return fail_after_final_token(
                api,
                &journal,
                &mut entry,
                &request.bootstrap_token,
                error,
                probe_cleanup_needed,
            );
        }
    };

    // The final token is now adopted by the committed config and must never
    // be revoked by journal recovery.
    entry.temporary_token_id = None;
    entry.final_token_id = None;
    entry.cleanup_token_id = None;
    entry.probe_cleanup_needed = false;
    entry.error_code = None;
    advance_journal(&mut entry, R2SetupStage::ConfigCommitted);
    // Removing the older snapshot is safe after config commit even if the
    // best-effort checkpoint cannot be written. Recovery also cross-checks
    // configured access IDs before any revocation.
    let _ = save_journal(&journal, &entry);
    journal
        .remove(&setup_id)
        .map_err(|_| StorageSetupError::persistence())?;

    Ok(ProvisionR2StorageResult {
        storage_id: connected.storage_id,
        config_revision: connected.config_revision,
        verified: connected.verified,
        bucket_created,
    })
}

fn cleanup_r2_setup_with(
    repository: &LocalStateRepository,
    request: CleanupR2SetupRequest,
    api: &impl CloudflareR2Api,
    probe: &impl CapabilityProbe,
    now: DateTime<Utc>,
) -> Result<CleanupR2SetupResult, StorageSetupError> {
    validate_bootstrap_token(&request.bootstrap_token)?;
    let setup_id = R2SetupId::parse(request.setup_id).map_err(|_| {
        StorageSetupError::invalid("r2_setup_id_invalid", "The R2 setup ID is invalid.")
    })?;
    let journal = R2SetupJournalStore::new(repository);
    let Some(mut entry) = journal
        .load(&setup_id)
        .map_err(|_| StorageSetupError::persistence())?
    else {
        return Ok(CleanupR2SetupResult {
            setup_id: setup_id.to_string(),
            cleaned: true,
        });
    };
    let jurisdiction = application_jurisdiction(entry.jurisdiction);
    let destination = destination_for(&entry.account_id, jurisdiction, &entry.bucket)?;

    if entry.probe_cleanup_needed || entry.stage == R2SetupStage::ProbeRunning {
        let configured = matching_configured_storage(repository, &destination)?;
        let mut cleanup_token_id: Option<String> = None;
        let mut cleanup_secret: Option<SecretString> = None;
        let mut reconciliation_storage = if let Some(storage) = configured {
            storage
        } else {
            if let Some(stale_cleanup_token_id) = entry.cleanup_token_id.clone() {
                if api
                    .revoke_token(
                        &entry.account_id,
                        &request.bootstrap_token,
                        &stale_cleanup_token_id,
                    )
                    .is_err()
                {
                    mark_cleanup_required(
                        &journal,
                        &mut entry,
                        "cleanup_token_revoke_failed",
                        true,
                    );
                    return Err(cleanup_required(setup_id.as_str()));
                }
                entry.cleanup_token_id = None;
                save_journal(&journal, &entry)?;
            }
            let permission = api.resolve_permission_group(
                &entry.account_id,
                &request.bootstrap_token,
                BUCKET_ITEM_WRITE,
                BUCKET_SCOPE,
            )?;
            let cleanup_token = api.create_token(
                &entry.account_id,
                &request.bootstrap_token,
                &cleanup_token_payload(
                    setup_id.as_str(),
                    &entry.account_id,
                    &entry.bucket,
                    jurisdiction,
                    permission,
                    now,
                ),
            )?;
            cleanup_token_id = Some(cleanup_token.id.clone());
            let derived = derive_s3_secret(&cleanup_token.value);
            cleanup_secret = Some(derived);
            entry.cleanup_token_id = Some(cleanup_token.id.clone());
            if save_journal(&journal, &entry).is_err() {
                let account_id = entry.account_id.clone();
                return revoke_after_unjournaled_creation(
                    api,
                    &account_id,
                    &request.bootstrap_token,
                    &cleanup_token.id,
                    &mut entry,
                    &journal,
                    true,
                );
            }
            StorageConfig {
                id: StorageId::parse("storage-r2-cleanup").map_err(|_| cloudflare_remote())?,
                name: "R2 setup cleanup".to_string(),
                kind: StorageKind::S3,
                bucket: entry.bucket.clone(),
                access_key_id: cleanup_token.id,
                secret_access_key: cleanup_secret
                    .as_ref()
                    .expect("cleanup secret exists")
                    .expose()
                    .to_string(),
                account_id: entry.account_id.clone(),
                s3_endpoint: destination.endpoint().to_string(),
                region: "auto".to_string(),
                local_dir: String::new(),
                supports_conditional_writes: Some(true),
            }
        };
        let reconciliation_result = probe.reconcile(&reconciliation_storage, setup_id.as_str());
        reconciliation_storage.secret_access_key.zeroize();
        if reconciliation_result.is_err() {
            drop(cleanup_secret);
            if let Some(token_id) = cleanup_token_id {
                if api
                    .revoke_token(&entry.account_id, &request.bootstrap_token, &token_id)
                    .is_ok()
                {
                    entry.cleanup_token_id = None;
                    mark_cleanup_required(&journal, &mut entry, "probe_cleanup_failed", true);
                } else {
                    mark_cleanup_required(
                        &journal,
                        &mut entry,
                        "probe_cleanup_and_token_revoke_failed",
                        true,
                    );
                }
            } else {
                mark_cleanup_required(&journal, &mut entry, "probe_cleanup_failed", true);
            }
            return Err(cleanup_required(setup_id.as_str()));
        }
        drop(cleanup_secret);
        entry.probe_cleanup_needed = false;
        if let Some(token_id) = cleanup_token_id {
            if api
                .revoke_token(&entry.account_id, &request.bootstrap_token, &token_id)
                .is_err()
            {
                mark_cleanup_required(&journal, &mut entry, "cleanup_token_revoke_failed", false);
                return Err(cleanup_required(setup_id.as_str()));
            }
            entry.cleanup_token_id = None;
        }
    }

    let configured_ids = configured_access_key_ids_for_destination(
        repository,
        &entry.account_id,
        jurisdiction,
        &entry.bucket,
    )?;
    for (role, token_id) in journal_token_ids(&entry) {
        if configured_ids.contains(&token_id) {
            clear_journal_token(&mut entry, role);
            continue;
        }
        if api
            .revoke_token(&entry.account_id, &request.bootstrap_token, &token_id)
            .is_err()
        {
            mark_cleanup_required(&journal, &mut entry, "token_revoke_failed", false);
            return Err(cleanup_required(setup_id.as_str()));
        }
        clear_journal_token(&mut entry, role);
        advance_updated_at(&mut entry, now);
        save_journal(&journal, &entry)?;
    }
    journal
        .remove(&setup_id)
        .map_err(|_| StorageSetupError::persistence())?;
    Ok(CleanupR2SetupResult {
        setup_id: setup_id.to_string(),
        cleaned: true,
    })
}

// The provisioned variant dwarfs the markers by design; boxing it would
// change every construction site for no behavioral gain.
#[allow(clippy::large_enum_variant)]
enum DestinationPreflight {
    Available,
    Connected {
        storage: StorageConfig,
        revision: u64,
    },
}

fn inspect_destination_preflight(
    repository: &LocalStateRepository,
    destination: &crate::storage_setup::R2Destination,
    requested_name: &str,
    expected_revision: Option<u64>,
) -> Result<DestinationPreflight, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    validate_r2_storage_destinations(&config)?;
    for storage in &config.storages {
        if storage.kind != StorageKind::S3 {
            continue;
        }
        if storage.s3_endpoint.is_empty() {
            return Err(StorageSetupError::persistence());
        }
        let stored = parse_r2_api_url(&format!("{}/{}", storage.s3_endpoint, storage.bucket))
            .map_err(|_| StorageSetupError::persistence())?;
        if stored.account_id() == destination.account_id()
            && stored.jurisdiction() == destination.jurisdiction()
            && stored.bucket() == destination.bucket()
        {
            if storage.name != requested_name {
                return Err(StorageSetupError::conflict(
                    "r2_destination_conflict",
                    "That R2 destination is already connected with a different name.",
                ));
            }
            return Ok(DestinationPreflight::Connected {
                storage: storage.clone(),
                revision: config.revision,
            });
        }
    }
    if expected_revision.is_some_and(|expected| expected != config.revision) {
        return Err(StorageSetupError::conflict(
            "config_revision_conflict",
            "Mallard storage configuration changed; reload it before provisioning storage.",
        ));
    }
    Ok(DestinationPreflight::Available)
}

#[allow(clippy::too_many_arguments)]
fn verify_idempotent_provision(
    _repository: &LocalStateRepository,
    journal: &R2SetupJournalStore,
    probe: &impl CapabilityProbe,
    mut storage: StorageConfig,
    revision: u64,
    account_id: &str,
    jurisdiction: R2Jurisdiction,
    bucket: &str,
    now: DateTime<Utc>,
) -> Result<ProvisionR2StorageResult, StorageSetupError> {
    let storage_id = storage.id.clone();
    let result = (|| {
        let setup_id = R2SetupId::parse(
            generated_named_id("r2setup").map_err(|_| StorageSetupError::persistence())?,
        )
        .map_err(|_| StorageSetupError::persistence())?;
        reconcile_previous_probes(
            journal,
            probe,
            &storage,
            account_id,
            jurisdiction,
            bucket,
            &setup_id,
        )?;
        let timestamp = nonzero_timestamp(now);
        let mut entry = R2SetupJournalEntry {
            schema: R2_SETUP_JOURNAL_SCHEMA,
            setup_id: setup_id.clone(),
            account_id: account_id.to_string(),
            bucket: bucket.to_string(),
            jurisdiction: journal_jurisdiction(jurisdiction),
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
        save_journal(journal, &entry)?;
        match probe.verify(&storage, setup_id.as_str()) {
            Ok(()) => {
                journal
                    .remove(&setup_id)
                    .map_err(|_| StorageSetupError::persistence())?;
                Ok(ProvisionR2StorageResult {
                    storage_id,
                    config_revision: revision,
                    verified: true,
                    bucket_created: false,
                })
            }
            Err(error) if error.kind() == StorageSetupErrorKind::CleanupRequired => {
                mark_cleanup_required(journal, &mut entry, error.code(), true);
                Err(cleanup_required(setup_id.as_str()))
            }
            Err(error) => {
                let _ = journal.remove(&setup_id);
                Err(error)
            }
        }
    })();
    storage.secret_access_key.zeroize();
    result
}

#[allow(clippy::too_many_arguments)]
fn reconcile_previous_tokens(
    repository: &LocalStateRepository,
    journal: &R2SetupJournalStore,
    api: &impl CloudflareR2Api,
    account_id: &str,
    jurisdiction: R2Jurisdiction,
    bucket: &str,
    bootstrap_token: &SecretString,
    now: DateTime<Utc>,
) -> Result<(), StorageSetupError> {
    let configured_ids =
        configured_access_key_ids_for_destination(repository, account_id, jurisdiction, bucket)?;
    let entries = journal
        .find_matching(account_id, journal_jurisdiction(jurisdiction), bucket)
        .map_err(|_| StorageSetupError::persistence())?;
    for mut entry in entries {
        for (role, token_id) in journal_token_ids(&entry) {
            if configured_ids.contains(&token_id) {
                clear_journal_token(&mut entry, role);
                continue;
            }
            if api
                .revoke_token(account_id, bootstrap_token, &token_id)
                .is_err()
            {
                mark_cleanup_required(journal, &mut entry, "token_revoke_failed", false);
                return Err(cleanup_required(entry.setup_id.as_str()));
            }
            clear_journal_token(&mut entry, role);
            advance_updated_at(&mut entry, now);
            save_journal(journal, &entry)?;
        }
        let possible_probe =
            entry.probe_cleanup_needed || entry.stage == R2SetupStage::ProbeRunning;
        if journal_tokens_empty(&entry) && !possible_probe {
            journal
                .remove(&entry.setup_id)
                .map_err(|_| StorageSetupError::persistence())?;
        } else {
            save_journal(journal, &entry)?;
        }
    }
    Ok(())
}

fn reconcile_previous_probes(
    journal: &R2SetupJournalStore,
    probe: &impl CapabilityProbe,
    storage: &StorageConfig,
    account_id: &str,
    jurisdiction: R2Jurisdiction,
    bucket: &str,
    current_setup_id: &R2SetupId,
) -> Result<(), StorageSetupError> {
    let entries = journal
        .find_matching(account_id, journal_jurisdiction(jurisdiction), bucket)
        .map_err(|_| StorageSetupError::persistence())?;
    for mut entry in entries {
        if &entry.setup_id == current_setup_id {
            continue;
        }
        if !entry.probe_cleanup_needed && entry.stage != R2SetupStage::ProbeRunning {
            continue;
        }
        if probe.reconcile(storage, entry.setup_id.as_str()).is_err() {
            mark_cleanup_required(journal, &mut entry, "probe_cleanup_failed", true);
            return Err(cleanup_required(entry.setup_id.as_str()));
        }
        entry.probe_cleanup_needed = false;
        entry.error_code = None;
        if journal_tokens_empty(&entry) {
            journal
                .remove(&entry.setup_id)
                .map_err(|_| StorageSetupError::persistence())?;
        } else {
            entry.stage = R2SetupStage::CleanupRequired;
            advance_updated_at(&mut entry, Utc::now());
            save_journal(journal, &entry)?;
        }
    }
    Ok(())
}

fn revoke_after_unjournaled_creation<T>(
    api: &impl CloudflareR2Api,
    account_id: &str,
    bootstrap_token: &SecretString,
    token_id: &str,
    entry: &mut R2SetupJournalEntry,
    journal: &R2SetupJournalStore,
    probe_cleanup_needed: bool,
) -> Result<T, StorageSetupError> {
    if api
        .revoke_token(account_id, bootstrap_token, token_id)
        .is_err()
    {
        mark_cleanup_required(journal, entry, "token_revoke_failed", probe_cleanup_needed);
        Err(cleanup_required(entry.setup_id.as_str()))
    } else {
        Err(StorageSetupError::persistence())
    }
}

fn fail_after_final_token(
    api: &impl CloudflareR2Api,
    journal: &R2SetupJournalStore,
    entry: &mut R2SetupJournalEntry,
    bootstrap_token: &SecretString,
    original: StorageSetupError,
    probe_cleanup_needed: bool,
) -> Result<ProvisionR2StorageResult, StorageSetupError> {
    let Some(final_token_id) = entry.final_token_id.clone() else {
        return Err(original);
    };
    if api
        .revoke_token(&entry.account_id, bootstrap_token, &final_token_id)
        .is_err()
    {
        mark_cleanup_required(
            journal,
            entry,
            "final_token_revoke_failed",
            probe_cleanup_needed,
        );
        return Err(cleanup_required(entry.setup_id.as_str()));
    }
    entry.final_token_id = None;
    if probe_cleanup_needed {
        mark_cleanup_required(journal, entry, original.code(), true);
    } else if journal_tokens_empty(entry) {
        journal
            .remove(&entry.setup_id)
            .map_err(|_| StorageSetupError::persistence())?;
    } else {
        save_journal(journal, entry)?;
    }
    Err(original)
}

fn mark_cleanup_required(
    journal: &R2SetupJournalStore,
    entry: &mut R2SetupJournalEntry,
    error_code: &'static str,
    probe_cleanup_needed: bool,
) {
    entry.stage = R2SetupStage::CleanupRequired;
    entry.error_code = Some(error_code.to_string());
    entry.probe_cleanup_needed = probe_cleanup_needed;
    entry.updated_at = entry.updated_at.saturating_add(1);
    let _ = journal.save(entry);
}

#[derive(Clone, Copy)]
enum JournalTokenRole {
    Temporary,
    Final,
    Cleanup,
}

fn journal_token_ids(entry: &R2SetupJournalEntry) -> Vec<(JournalTokenRole, String)> {
    [
        (
            JournalTokenRole::Temporary,
            entry.temporary_token_id.as_ref(),
        ),
        (JournalTokenRole::Final, entry.final_token_id.as_ref()),
        (JournalTokenRole::Cleanup, entry.cleanup_token_id.as_ref()),
    ]
    .into_iter()
    .filter_map(|(role, token_id)| token_id.cloned().map(|token_id| (role, token_id)))
    .collect()
}

fn clear_journal_token(entry: &mut R2SetupJournalEntry, role: JournalTokenRole) {
    match role {
        JournalTokenRole::Temporary => entry.temporary_token_id = None,
        JournalTokenRole::Final => entry.final_token_id = None,
        JournalTokenRole::Cleanup => entry.cleanup_token_id = None,
    }
}

fn journal_tokens_empty(entry: &R2SetupJournalEntry) -> bool {
    entry.temporary_token_id.is_none()
        && entry.final_token_id.is_none()
        && entry.cleanup_token_id.is_none()
}

fn save_journal(
    journal: &R2SetupJournalStore,
    entry: &R2SetupJournalEntry,
) -> Result<(), StorageSetupError> {
    journal
        .save(entry)
        .map_err(|_| StorageSetupError::persistence())
}

fn advance_journal(entry: &mut R2SetupJournalEntry, stage: R2SetupStage) {
    entry.stage = stage;
    entry.error_code = None;
    entry.probe_cleanup_needed = false;
    entry.updated_at = entry.updated_at.saturating_add(1);
}

fn advance_updated_at(entry: &mut R2SetupJournalEntry, now: DateTime<Utc>) {
    entry.updated_at = entry
        .updated_at
        .saturating_add(1)
        .max(nonzero_timestamp(now));
}

fn configured_access_key_ids_for_destination(
    repository: &LocalStateRepository,
    account_id: &str,
    jurisdiction: R2Jurisdiction,
    bucket: &str,
) -> Result<BTreeSet<String>, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    let destination = destination_for(account_id, jurisdiction, bucket)?;
    let mut access_ids = BTreeSet::new();
    for storage in config.storages {
        if storage.kind != StorageKind::S3 {
            continue;
        }
        if storage.s3_endpoint.is_empty() {
            return Err(StorageSetupError::persistence());
        }
        let stored = parse_r2_api_url(&format!("{}/{}", storage.s3_endpoint, storage.bucket))
            .map_err(|_| StorageSetupError::persistence())?;
        if stored.account_id() == destination.account_id()
            && stored.jurisdiction() == destination.jurisdiction()
            && stored.bucket() == destination.bucket()
        {
            access_ids.insert(storage.access_key_id);
        }
    }
    Ok(access_ids)
}

fn matching_configured_storage(
    repository: &LocalStateRepository,
    destination: &crate::storage_setup::R2Destination,
) -> Result<Option<StorageConfig>, StorageSetupError> {
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    for storage in config.storages {
        if storage.kind != StorageKind::S3 {
            continue;
        }
        if storage.s3_endpoint.is_empty() {
            return Err(StorageSetupError::persistence());
        }
        let parsed = parse_r2_api_url(&format!("{}/{}", storage.s3_endpoint, storage.bucket))
            .map_err(|_| StorageSetupError::persistence())?;
        if parsed.account_id() == destination.account_id()
            && parsed.jurisdiction() == destination.jurisdiction()
            && parsed.bucket() == destination.bucket()
        {
            return Ok(Some(storage));
        }
    }
    Ok(None)
}

fn destination_for(
    account_id: &str,
    jurisdiction: R2Jurisdiction,
    bucket: &str,
) -> Result<crate::storage_setup::R2Destination, StorageSetupError> {
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
    parse_r2_api_url(&format!("{endpoint}/{bucket}"))
}

fn canonical_account_id(value: &str) -> Result<String, StorageSetupError> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageSetupError::invalid(
            "r2_account_id_invalid",
            "The Cloudflare account ID must be 32 hexadecimal characters.",
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_storage_name(value: &str) -> Result<(), StorageSetupError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 1_024
        || value.chars().any(char::is_control)
    {
        Err(StorageSetupError::invalid(
            "storage_name_invalid",
            "The storage name is invalid.",
        ))
    } else {
        Ok(())
    }
}

fn validate_bootstrap_token(token: &SecretString) -> Result<(), StorageSetupError> {
    if token.expose().is_empty()
        || token.expose().len() > MAX_BOOTSTRAP_TOKEN_BYTES
        || token.expose().chars().any(char::is_control)
    {
        Err(StorageSetupError::invalid(
            "cloudflare_bootstrap_token_invalid",
            "The Cloudflare bootstrap token is invalid.",
        ))
    } else {
        Ok(())
    }
}

fn journal_jurisdiction(value: R2Jurisdiction) -> R2SetupJurisdiction {
    match value {
        R2Jurisdiction::Default => R2SetupJurisdiction::Default,
        R2Jurisdiction::Eu => R2SetupJurisdiction::Eu,
        R2Jurisdiction::Fedramp => R2SetupJurisdiction::Fedramp,
    }
}

fn application_jurisdiction(value: R2SetupJurisdiction) -> R2Jurisdiction {
    match value {
        R2SetupJurisdiction::Default => R2Jurisdiction::Default,
        R2SetupJurisdiction::Eu => R2Jurisdiction::Eu,
        R2SetupJurisdiction::Fedramp => R2Jurisdiction::Fedramp,
    }
}

fn nonzero_timestamp(now: DateTime<Utc>) -> u64 {
    u64::try_from(now.timestamp()).unwrap_or(1).max(1)
}

struct CloudflareHttpClient {
    client: Client,
}

impl CloudflareHttpClient {
    fn new() -> Result<Self, StorageSetupError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(RedirectPolicy::none())
            .https_only(true)
            .user_agent("mallard-r2-setup/1")
            .build()
            .map_err(|_| cloudflare_remote())?;
        Ok(Self { client })
    }

    fn bearer(&self, request: RequestBuilder, token: &SecretString) -> RequestBuilder {
        request.bearer_auth(token.expose())
    }

    fn json_result<T: DeserializeOwned>(&self, response: Response) -> Result<T, StorageSetupError> {
        if !response.status().is_success() {
            return Err(cloudflare_remote());
        }
        let envelope: CloudflareEnvelope<T> = read_bounded_json(response)?;
        if envelope.success != Some(true) {
            return Err(cloudflare_remote());
        }
        envelope.result.ok_or_else(cloudflare_remote)
    }

    fn success_without_result(&self, response: Response) -> Result<(), StorageSetupError> {
        if !response.status().is_success() {
            return Err(cloudflare_remote());
        }
        let envelope: CloudflareEnvelope<Value> = read_bounded_json(response)?;
        if envelope.success == Some(true) {
            Ok(())
        } else {
            Err(cloudflare_remote())
        }
    }
}

#[derive(Deserialize)]
struct CloudflareEnvelope<T> {
    success: Option<bool>,
    result: Option<T>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct PermissionGroup {
    id: String,
    name: String,
    scopes: Vec<String>,
}

struct CreatedToken {
    id: String,
    value: SecretString,
}

#[derive(Deserialize)]
struct CreatedTokenResponse {
    id: Option<String>,
    value: Option<SecretString>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct BucketResponse {
    name: Option<String>,
    jurisdiction: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BucketStatus {
    Present,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BucketCreateStatus {
    Created,
    Conflict,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct TokenCreatePayload {
    name: String,
    policies: Vec<TokenPolicyPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_on: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct TokenPolicyPayload {
    effect: &'static str,
    permission_groups: Vec<PermissionGroupPayload>,
    resources: Value,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct PermissionGroupPayload {
    id: String,
}

trait CloudflareR2Api {
    fn resolve_permission_group(
        &self,
        account_id: &str,
        bootstrap_token: &SecretString,
        name: &str,
        scope: &str,
    ) -> Result<String, StorageSetupError>;

    fn create_token(
        &self,
        account_id: &str,
        bootstrap_token: &SecretString,
        payload: &TokenCreatePayload,
    ) -> Result<CreatedToken, StorageSetupError>;

    fn revoke_token(
        &self,
        account_id: &str,
        bootstrap_token: &SecretString,
        token_id: &str,
    ) -> Result<(), StorageSetupError>;

    fn get_bucket(
        &self,
        account_id: &str,
        r2_token: &SecretString,
        bucket: &str,
        jurisdiction: R2Jurisdiction,
    ) -> Result<BucketStatus, StorageSetupError>;

    fn create_bucket(
        &self,
        account_id: &str,
        r2_token: &SecretString,
        bucket: &str,
        jurisdiction: R2Jurisdiction,
    ) -> Result<BucketCreateStatus, StorageSetupError>;
}

impl CloudflareR2Api for CloudflareHttpClient {
    fn resolve_permission_group(
        &self,
        account_id: &str,
        bootstrap_token: &SecretString,
        name: &str,
        scope: &str,
    ) -> Result<String, StorageSetupError> {
        let url = format!("{CLOUDFLARE_API_BASE}/accounts/{account_id}/tokens/permission_groups");
        let response = self
            .bearer(
                self.client
                    .get(url)
                    .query(&[("name", name), ("scope", scope)]),
                bootstrap_token,
            )
            .send()
            .map_err(|_| cloudflare_remote())?;
        let groups: Vec<PermissionGroup> = self.json_result(response)?;
        resolve_permission_group(groups, name, scope)
    }

    fn create_token(
        &self,
        account_id: &str,
        bootstrap_token: &SecretString,
        payload: &TokenCreatePayload,
    ) -> Result<CreatedToken, StorageSetupError> {
        let body = serde_json::to_vec(payload).map_err(|_| cloudflare_remote())?;
        if body.len() > 64 * 1024 {
            return Err(cloudflare_remote());
        }
        let url = format!("{CLOUDFLARE_API_BASE}/accounts/{account_id}/tokens");
        let response = self
            .bearer(
                self.client
                    .post(url)
                    .header("content-type", "application/json")
                    .body(body),
                bootstrap_token,
            )
            .send()
            .map_err(|_| cloudflare_remote())?;
        let result: CreatedTokenResponse = self.json_result(response)?;
        let id = result.id.ok_or_else(cloudflare_remote)?;
        let value = result.value.ok_or_else(cloudflare_remote)?;
        validate_cloudflare_id(&id)?;
        if !(40..=80).contains(&value.expose().len())
            || value.expose().chars().any(char::is_control)
        {
            return Err(cloudflare_remote());
        }
        Ok(CreatedToken { id, value })
    }

    fn revoke_token(
        &self,
        account_id: &str,
        bootstrap_token: &SecretString,
        token_id: &str,
    ) -> Result<(), StorageSetupError> {
        validate_cloudflare_id(token_id)?;
        let url = format!("{CLOUDFLARE_API_BASE}/accounts/{account_id}/tokens/{token_id}");
        let response = self
            .bearer(self.client.delete(url), bootstrap_token)
            .send()
            .map_err(|_| cloudflare_remote())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            // A missing token is already in the desired revoked state.
            return Ok(());
        }
        self.success_without_result(response)
    }

    fn get_bucket(
        &self,
        account_id: &str,
        r2_token: &SecretString,
        bucket: &str,
        jurisdiction: R2Jurisdiction,
    ) -> Result<BucketStatus, StorageSetupError> {
        let url = format!("{CLOUDFLARE_API_BASE}/accounts/{account_id}/r2/buckets/{bucket}");
        let response = self
            .bearer(
                self.client
                    .get(url)
                    .header("cf-r2-jurisdiction", jurisdiction.as_str()),
                r2_token,
            )
            .send()
            .map_err(|_| cloudflare_remote())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(BucketStatus::Missing);
        }
        let result: BucketResponse = self.json_result(response)?;
        validate_bucket_response(&result, bucket, jurisdiction)?;
        Ok(BucketStatus::Present)
    }

    fn create_bucket(
        &self,
        account_id: &str,
        r2_token: &SecretString,
        bucket: &str,
        jurisdiction: R2Jurisdiction,
    ) -> Result<BucketCreateStatus, StorageSetupError> {
        let url = format!("{CLOUDFLARE_API_BASE}/accounts/{account_id}/r2/buckets");
        let body =
            serde_json::to_vec(&json!({ "name": bucket })).map_err(|_| cloudflare_remote())?;
        let response = self
            .bearer(
                self.client
                    .post(url)
                    .header("content-type", "application/json")
                    .header("cf-r2-jurisdiction", jurisdiction.as_str())
                    .body(body),
                r2_token,
            )
            .send()
            .map_err(|_| cloudflare_remote())?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Ok(BucketCreateStatus::Conflict);
        }
        let result: BucketResponse = self.json_result(response)?;
        validate_bucket_response(&result, bucket, jurisdiction)?;
        Ok(BucketCreateStatus::Created)
    }
}

fn read_bounded_json<T: DeserializeOwned>(response: Response) -> Result<T, StorageSetupError> {
    if !has_json_content_type(response.headers()) {
        return Err(cloudflare_remote());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CLOUDFLARE_RESPONSE_BYTES)
    {
        return Err(cloudflare_remote());
    }
    let mut bytes = Vec::new();
    let read_result = response
        .take(MAX_CLOUDFLARE_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes);
    if read_result.is_err() {
        bytes.zeroize();
        return Err(cloudflare_remote());
    }
    if bytes.len() as u64 > MAX_CLOUDFLARE_RESPONSE_BYTES {
        bytes.zeroize();
        return Err(cloudflare_remote());
    }
    let result = serde_json::from_slice(&bytes).map_err(|_| cloudflare_remote());
    bytes.zeroize();
    result
}

fn has_json_content_type(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn resolve_permission_group(
    groups: Vec<PermissionGroup>,
    expected_name: &str,
    expected_scope: &str,
) -> Result<String, StorageSetupError> {
    let matches = groups
        .into_iter()
        .filter(|group| {
            group.name == expected_name
                && group.scopes.len() == 1
                && group.scopes[0] == expected_scope
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(cloudflare_remote());
    }
    validate_cloudflare_id(&matches[0].id)?;
    Ok(matches[0].id.clone())
}

fn temporary_token_payload(
    setup_id: &str,
    account_id: &str,
    permission_group_id: String,
    now: DateTime<Utc>,
) -> TokenCreatePayload {
    TokenCreatePayload {
        name: format!("Mallard R2 setup {setup_id} temporary"),
        policies: vec![TokenPolicyPayload {
            effect: "allow",
            permission_groups: vec![PermissionGroupPayload {
                id: permission_group_id,
            }],
            resources: json!({
                format!("com.cloudflare.api.account.{account_id}"): {
                    "com.cloudflare.edge.r2.bucket.*": "*"
                }
            }),
        }],
        expires_on: Some(
            (now + ChronoDuration::minutes(TEMPORARY_TOKEN_TTL_MINUTES))
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        ),
    }
}

fn final_token_payload(
    setup_id: &str,
    account_id: &str,
    bucket: &str,
    jurisdiction: R2Jurisdiction,
    permission_group_id: String,
) -> TokenCreatePayload {
    TokenCreatePayload {
        name: format!("Mallard R2 storage {setup_id}"),
        policies: vec![TokenPolicyPayload {
            effect: "allow",
            permission_groups: vec![PermissionGroupPayload {
                id: permission_group_id,
            }],
            resources: json!({
                format!(
                    "com.cloudflare.edge.r2.bucket.{account_id}_{}_{bucket}",
                    jurisdiction.as_str()
                ): "*"
            }),
        }],
        expires_on: None,
    }
}

fn cleanup_token_payload(
    setup_id: &str,
    account_id: &str,
    bucket: &str,
    jurisdiction: R2Jurisdiction,
    permission_group_id: String,
    now: DateTime<Utc>,
) -> TokenCreatePayload {
    let mut payload = final_token_payload(
        setup_id,
        account_id,
        bucket,
        jurisdiction,
        permission_group_id,
    );
    payload.name = format!("Mallard R2 cleanup {setup_id}");
    payload.expires_on = Some(
        (now + ChronoDuration::minutes(TEMPORARY_TOKEN_TTL_MINUTES))
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    );
    payload
}

fn validate_bucket_response(
    response: &BucketResponse,
    bucket: &str,
    jurisdiction: R2Jurisdiction,
) -> Result<(), StorageSetupError> {
    if response.name.as_deref() != Some(bucket)
        || response.jurisdiction.as_deref() != Some(jurisdiction.as_str())
    {
        Err(cloudflare_remote())
    } else {
        Ok(())
    }
}

fn validate_cloudflare_id(value: &str) -> Result<(), StorageSetupError> {
    if value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(cloudflare_remote())
    }
}

fn derive_s3_secret(token_value: &SecretString) -> SecretString {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut digest = Sha256::digest(token_value.expose().as_bytes());
    let mut output = String::with_capacity(64);
    for byte in &digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    digest.as_mut_slice().zeroize();
    SecretString::new(output)
}

fn cloudflare_remote() -> StorageSetupError {
    StorageSetupError::fixed(
        StorageSetupErrorKind::RemoteFailure,
        "cloudflare_api_failed",
        "Cloudflare authentication or R2 provisioning failed.",
    )
}

fn cleanup_required(setup_id: &str) -> StorageSetupError {
    StorageSetupError::cleanup_required(setup_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mallard_domain::SyncConfig;
    use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeSet, VecDeque};

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
    const ACCOUNT_PERMISSION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BUCKET_PERMISSION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TEMPORARY_ID: &str = "cccccccccccccccccccccccccccccccc";
    const FINAL_ID: &str = "dddddddddddddddddddddddddddddddd";
    const CLEANUP_ID: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const SECOND_CLEANUP_ID: &str = "ffffffffffffffffffffffffffffffff";
    const BOOTSTRAP_CANARY: &str = "bootstrap-token-canary-never-persist";
    const TEMPORARY_VALUE: &str = "temporary-token-value-canary-00000000000000000000";
    const FINAL_VALUE: &str = "final-token-value-canary-000000000000000000000000";
    const CLEANUP_VALUE: &str = "cleanup-token-value-canary-000000000000000000000";

    #[derive(Clone, Copy)]
    enum ProbeOutcome {
        Success,
        RemoteFailure,
        CleanupRequired,
    }

    struct FakeProbe {
        verify_outcome: Cell<ProbeOutcome>,
        reconcile_outcome: Cell<ProbeOutcome>,
        verify_calls: Cell<usize>,
        reconcile_calls: Cell<usize>,
    }

    impl FakeProbe {
        fn successful() -> Self {
            Self {
                verify_outcome: Cell::new(ProbeOutcome::Success),
                reconcile_outcome: Cell::new(ProbeOutcome::Success),
                verify_calls: Cell::new(0),
                reconcile_calls: Cell::new(0),
            }
        }

        fn outcome(outcome: ProbeOutcome) -> Self {
            Self {
                verify_outcome: Cell::new(outcome),
                ..Self::successful()
            }
        }

        fn result(outcome: ProbeOutcome, setup_id: &str) -> Result<(), StorageSetupError> {
            match outcome {
                ProbeOutcome::Success => Ok(()),
                ProbeOutcome::RemoteFailure => Err(StorageSetupError::remote()),
                ProbeOutcome::CleanupRequired => Err(StorageSetupError::cleanup_required(setup_id)),
            }
        }
    }

    impl CapabilityProbe for FakeProbe {
        fn verify(
            &self,
            _storage: &StorageConfig,
            setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.verify_calls.set(self.verify_calls.get() + 1);
            Self::result(self.verify_outcome.get(), setup_id)
        }

        fn reconcile(
            &self,
            _storage: &StorageConfig,
            setup_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.reconcile_calls.set(self.reconcile_calls.get() + 1);
            Self::result(self.reconcile_outcome.get(), setup_id)
        }
    }

    struct FakeApi {
        tokens: RefCell<VecDeque<(String, String)>>,
        payloads: RefCell<Vec<TokenCreatePayload>>,
        permission_requests: RefCell<Vec<(String, String)>>,
        bucket_reads: RefCell<VecDeque<BucketStatus>>,
        bucket_create: RefCell<BucketCreateStatus>,
        revoked: RefCell<Vec<String>>,
        revoke_failures: RefCell<BTreeSet<String>>,
    }

    impl FakeApi {
        fn new(tokens: &[(&str, &str)]) -> Self {
            Self {
                tokens: RefCell::new(
                    tokens
                        .iter()
                        .map(|(id, value)| ((*id).to_string(), (*value).to_string()))
                        .collect(),
                ),
                payloads: RefCell::new(Vec::new()),
                permission_requests: RefCell::new(Vec::new()),
                bucket_reads: RefCell::new(VecDeque::from([BucketStatus::Present])),
                bucket_create: RefCell::new(BucketCreateStatus::Created),
                revoked: RefCell::new(Vec::new()),
                revoke_failures: RefCell::new(BTreeSet::new()),
            }
        }
    }

    impl CloudflareR2Api for FakeApi {
        fn resolve_permission_group(
            &self,
            _account_id: &str,
            _bootstrap_token: &SecretString,
            name: &str,
            scope: &str,
        ) -> Result<String, StorageSetupError> {
            self.permission_requests
                .borrow_mut()
                .push((name.to_string(), scope.to_string()));
            match (name, scope) {
                (ACCOUNT_R2_WRITE, ACCOUNT_SCOPE) => Ok(ACCOUNT_PERMISSION.to_string()),
                (BUCKET_ITEM_WRITE, BUCKET_SCOPE) => Ok(BUCKET_PERMISSION.to_string()),
                _ => Err(cloudflare_remote()),
            }
        }

        fn create_token(
            &self,
            _account_id: &str,
            _bootstrap_token: &SecretString,
            payload: &TokenCreatePayload,
        ) -> Result<CreatedToken, StorageSetupError> {
            self.payloads.borrow_mut().push(payload.clone());
            let (id, value) = self
                .tokens
                .borrow_mut()
                .pop_front()
                .ok_or_else(cloudflare_remote)?;
            Ok(CreatedToken {
                id,
                value: SecretString::new(value),
            })
        }

        fn revoke_token(
            &self,
            _account_id: &str,
            _bootstrap_token: &SecretString,
            token_id: &str,
        ) -> Result<(), StorageSetupError> {
            self.revoked.borrow_mut().push(token_id.to_string());
            if self.revoke_failures.borrow().contains(token_id) {
                Err(cloudflare_remote())
            } else {
                Ok(())
            }
        }

        fn get_bucket(
            &self,
            _account_id: &str,
            _r2_token: &SecretString,
            _bucket: &str,
            _jurisdiction: R2Jurisdiction,
        ) -> Result<BucketStatus, StorageSetupError> {
            self.bucket_reads
                .borrow_mut()
                .pop_front()
                .ok_or_else(cloudflare_remote)
        }

        fn create_bucket(
            &self,
            _account_id: &str,
            _r2_token: &SecretString,
            _bucket: &str,
            _jurisdiction: R2Jurisdiction,
        ) -> Result<BucketCreateStatus, StorageSetupError> {
            Ok(self.bucket_create.borrow().clone())
        }
    }

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path()).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).single().unwrap()
    }

    fn provision_request(name: &str) -> ProvisionR2StorageRequest {
        ProvisionR2StorageRequest {
            name: name.to_string(),
            account_id: ACCOUNT.to_string(),
            bucket: "team-backup".to_string(),
            jurisdiction: R2Jurisdiction::Default,
            bootstrap_token: SecretString::new(BOOTSTRAP_CANARY.to_string()),
            expected_config_revision: Some(0),
        }
    }

    fn journal_entry(setup_id: &str, stage: R2SetupStage) -> R2SetupJournalEntry {
        R2SetupJournalEntry {
            schema: R2_SETUP_JOURNAL_SCHEMA,
            setup_id: R2SetupId::parse(setup_id).unwrap(),
            account_id: ACCOUNT.to_string(),
            bucket: "team-backup".to_string(),
            jurisdiction: R2SetupJurisdiction::Default,
            stage,
            temporary_token_id: None,
            final_token_id: None,
            cleanup_token_id: None,
            bucket_created: false,
            created_at: 1,
            updated_at: 1,
            error_code: None,
            probe_cleanup_needed: false,
        }
    }

    #[test]
    fn token_policies_are_exact_and_cleanup_credentials_expire() {
        let fixed_now = now();
        let temporary = temporary_token_payload(
            "r2setup-111111111111111111111111",
            ACCOUNT,
            ACCOUNT_PERMISSION.to_string(),
            fixed_now,
        );
        let final_token = final_token_payload(
            "r2setup-111111111111111111111111",
            ACCOUNT,
            "team-backup",
            R2Jurisdiction::Eu,
            BUCKET_PERMISSION.to_string(),
        );
        let cleanup = cleanup_token_payload(
            "r2setup-111111111111111111111111",
            ACCOUNT,
            "team-backup",
            R2Jurisdiction::Eu,
            BUCKET_PERMISSION.to_string(),
            fixed_now,
        );

        let expiry =
            (fixed_now + ChronoDuration::minutes(15)).to_rfc3339_opts(SecondsFormat::Secs, true);
        assert_eq!(temporary.expires_on.as_deref(), Some(expiry.as_str()));
        assert_eq!(cleanup.expires_on.as_deref(), Some(expiry.as_str()));
        assert_eq!(final_token.expires_on, None);
        assert_eq!(
            temporary.policies[0].resources,
            json!({
                format!("com.cloudflare.api.account.{ACCOUNT}"): {
                    "com.cloudflare.edge.r2.bucket.*": "*"
                }
            })
        );
        assert_eq!(
            final_token.policies[0].resources,
            json!({
                format!("com.cloudflare.edge.r2.bucket.{ACCOUNT}_eu_team-backup"): "*"
            })
        );
        assert_eq!(cleanup.policies, final_token.policies);
    }

    #[test]
    fn permission_resolution_requires_one_exact_name_scope_and_lowercase_id() {
        let exact = PermissionGroup {
            id: ACCOUNT_PERMISSION.to_string(),
            name: ACCOUNT_R2_WRITE.to_string(),
            scopes: vec![ACCOUNT_SCOPE.to_string()],
        };
        assert_eq!(
            resolve_permission_group(vec![exact.clone()], ACCOUNT_R2_WRITE, ACCOUNT_SCOPE).unwrap(),
            ACCOUNT_PERMISSION
        );
        assert!(resolve_permission_group(
            vec![exact.clone(), exact.clone()],
            ACCOUNT_R2_WRITE,
            ACCOUNT_SCOPE
        )
        .is_err());
        let mut wrong_scope = exact.clone();
        wrong_scope.scopes = vec![BUCKET_SCOPE.to_string()];
        assert!(
            resolve_permission_group(vec![wrong_scope], ACCOUNT_R2_WRITE, ACCOUNT_SCOPE).is_err()
        );
        let mut uppercase_id = exact;
        uppercase_id.id = ACCOUNT_PERMISSION.to_uppercase();
        assert!(
            resolve_permission_group(vec![uppercase_id], ACCOUNT_R2_WRITE, ACCOUNT_SCOPE).is_err()
        );
    }

    #[test]
    fn response_content_type_requires_application_json() {
        let mut headers = HeaderMap::new();
        assert!(!has_json_content_type(&headers));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/json"));
        assert!(!has_json_content_type(&headers));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(has_json_content_type(&headers));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("Application/JSON"));
        assert!(has_json_content_type(&headers));
    }

    #[test]
    fn token_value_derivation_matches_sha256_vector() {
        let token = SecretString::new("abc".to_string());
        let derived = derive_s3_secret(&token);
        assert_eq!(
            derived.expose(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn provision_existing_bucket_commits_only_derived_bucket_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let api = FakeApi::new(&[(TEMPORARY_ID, TEMPORARY_VALUE), (FINAL_ID, FINAL_VALUE)]);
        let probe = FakeProbe::successful();

        let result = provision_r2_storage_with(
            &repository,
            provision_request("Team backup"),
            &api,
            &probe,
            now(),
        )
        .unwrap();

        assert!(result.verified);
        assert!(!result.bucket_created);
        assert_eq!(result.config_revision, 1);
        assert_eq!(probe.verify_calls.get(), 1);
        assert_eq!(api.revoked.borrow().as_slice(), &[TEMPORARY_ID]);
        let config = repository.load_config().unwrap();
        assert_eq!(config.storages.len(), 1);
        assert_eq!(config.storages[0].access_key_id, FINAL_ID);
        let expected = derive_s3_secret(&SecretString::new(FINAL_VALUE.to_string()));
        assert_eq!(config.storages[0].secret_access_key, expected.expose());
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());

        let serialized = std::fs::read_to_string(repository.config_path()).unwrap();
        for canary in [BOOTSTRAP_CANARY, TEMPORARY_VALUE, FINAL_VALUE] {
            assert!(
                !serialized.contains(canary),
                "persisted secret canary {canary}"
            );
        }
        let payloads = api.payloads.borrow();
        assert_eq!(payloads.len(), 2);
        assert!(payloads[0].expires_on.is_some());
        assert!(payloads[1].expires_on.is_none());
    }

    #[test]
    fn provision_handles_created_bucket_and_conflict_reread() {
        for (create_status, reads, expected_created) in [
            (
                BucketCreateStatus::Created,
                VecDeque::from([BucketStatus::Missing]),
                true,
            ),
            (
                BucketCreateStatus::Conflict,
                VecDeque::from([BucketStatus::Missing, BucketStatus::Present]),
                false,
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let repository = repository(&temp);
            let api = FakeApi::new(&[(TEMPORARY_ID, TEMPORARY_VALUE), (FINAL_ID, FINAL_VALUE)]);
            *api.bucket_create.borrow_mut() = create_status;
            *api.bucket_reads.borrow_mut() = reads;
            let result = provision_r2_storage_with(
                &repository,
                provision_request("Team backup"),
                &api,
                &FakeProbe::successful(),
                now(),
            )
            .unwrap();
            assert_eq!(result.bucket_created, expected_created);
        }
    }

    #[test]
    fn noncanonical_existing_s3_entry_blocks_provision_before_cloudflare_work() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let mut config = SyncConfig::default();
        config.storages.push(StorageConfig {
            id: StorageId::parse("storage-generic-existing").unwrap(),
            name: "Generic S3".to_string(),
            kind: StorageKind::S3,
            bucket: "legacy-bucket".to_string(),
            access_key_id: "legacy-access".to_string(),
            secret_access_key: "legacy-secret".to_string(),
            account_id: ACCOUNT.to_string(),
            s3_endpoint: "https://s3.example.com".to_string(),
            region: "auto".to_string(),
            local_dir: String::new(),
            supports_conditional_writes: Some(true),
        });
        repository.save_config(config).unwrap();
        let api = FakeApi::new(&[]);
        let probe = FakeProbe::successful();

        let error = provision_r2_storage_with(
            &repository,
            provision_request("Team backup"),
            &api,
            &probe,
            now(),
        )
        .unwrap_err();

        assert_eq!(error.code(), "r2_storage_destination_invalid");
        assert!(api.permission_requests.borrow().is_empty());
        assert!(api.payloads.borrow().is_empty());
        assert_eq!(probe.verify_calls.get(), 0);
    }

    #[test]
    fn failed_probe_revokes_unadopted_tokens_and_never_commits() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let api = FakeApi::new(&[(TEMPORARY_ID, TEMPORARY_VALUE), (FINAL_ID, FINAL_VALUE)]);
        let error = provision_r2_storage_with(
            &repository,
            provision_request("Team backup"),
            &api,
            &FakeProbe::outcome(ProbeOutcome::RemoteFailure),
            now(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::RemoteFailure);
        assert_eq!(api.revoked.borrow().as_slice(), &[TEMPORARY_ID, FINAL_ID]);
        assert!(repository.load_config().unwrap().storages.is_empty());
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
        let output = format!("{error:?} {error}");
        for canary in [ACCOUNT, "team-backup", BOOTSTRAP_CANARY, FINAL_VALUE] {
            assert!(!output.contains(canary), "error leaked {canary}");
        }
    }

    #[test]
    fn probe_cleanup_requirement_survives_after_final_token_revocation() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let api = FakeApi::new(&[(TEMPORARY_ID, TEMPORARY_VALUE), (FINAL_ID, FINAL_VALUE)]);
        let error = provision_r2_storage_with(
            &repository,
            provision_request("Team backup"),
            &api,
            &FakeProbe::outcome(ProbeOutcome::CleanupRequired),
            now(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::CleanupRequired);
        assert_eq!(api.revoked.borrow().as_slice(), &[TEMPORARY_ID, FINAL_ID]);
        let entries = R2SetupJournalStore::new(&repository).list().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].probe_cleanup_needed);
        assert!(journal_tokens_empty(&entries[0]));
        assert!(repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn failed_final_token_revocation_retains_only_recoverable_identifiers() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let api = FakeApi::new(&[(TEMPORARY_ID, TEMPORARY_VALUE), (FINAL_ID, FINAL_VALUE)]);
        api.revoke_failures
            .borrow_mut()
            .insert(FINAL_ID.to_string());
        let error = provision_r2_storage_with(
            &repository,
            provision_request("Team backup"),
            &api,
            &FakeProbe::outcome(ProbeOutcome::RemoteFailure),
            now(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), StorageSetupErrorKind::CleanupRequired);
        let entries = R2SetupJournalStore::new(&repository).list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].final_token_id.as_deref(), Some(FINAL_ID));
        let serialized = std::fs::read_to_string(
            R2SetupJournalStore::new(&repository)
                .directory()
                .join(format!("{}.json", entries[0].setup_id)),
        )
        .unwrap();
        for canary in [BOOTSTRAP_CANARY, TEMPORARY_VALUE, FINAL_VALUE] {
            assert!(!serialized.contains(canary), "journal leaked {canary}");
        }
    }

    #[test]
    fn cleanup_reconciliation_failure_revokes_short_lived_cleanup_token() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let setup_id = "r2setup-111111111111111111111111";
        let mut entry = journal_entry(setup_id, R2SetupStage::CleanupRequired);
        entry.probe_cleanup_needed = true;
        entry.error_code = Some("probe_cleanup_failed".to_string());
        R2SetupJournalStore::new(&repository).save(&entry).unwrap();
        let api = FakeApi::new(&[
            (CLEANUP_ID, CLEANUP_VALUE),
            (SECOND_CLEANUP_ID, CLEANUP_VALUE),
        ]);
        let probe = FakeProbe::successful();
        probe.reconcile_outcome.set(ProbeOutcome::RemoteFailure);

        let error = cleanup_r2_setup_with(
            &repository,
            CleanupR2SetupRequest {
                setup_id: setup_id.to_string(),
                bootstrap_token: SecretString::new(BOOTSTRAP_CANARY.to_string()),
            },
            &api,
            &probe,
            now(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), StorageSetupErrorKind::CleanupRequired);
        assert_eq!(api.revoked.borrow().as_slice(), &[CLEANUP_ID]);
        let retained = R2SetupJournalStore::new(&repository)
            .load(&R2SetupId::parse(setup_id).unwrap())
            .unwrap()
            .unwrap();
        assert!(retained.cleanup_token_id.is_none());
        assert!(retained.probe_cleanup_needed);
        assert!(api.payloads.borrow()[0].expires_on.is_some());

        probe.reconcile_outcome.set(ProbeOutcome::Success);
        let result = cleanup_r2_setup_with(
            &repository,
            CleanupR2SetupRequest {
                setup_id: setup_id.to_string(),
                bootstrap_token: SecretString::new(BOOTSTRAP_CANARY.to_string()),
            },
            &api,
            &probe,
            now(),
        )
        .unwrap();
        assert!(result.cleaned);
        assert_eq!(
            api.revoked.borrow().as_slice(),
            &[CLEANUP_ID, SECOND_CLEANUP_ID]
        );
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn committed_final_token_is_never_revoked_during_crash_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let repository = repository(&temp);
        let mut config = SyncConfig::default();
        config.storages.push(StorageConfig {
            id: StorageId::parse("storage-committed-r2").unwrap(),
            name: "Team backup".to_string(),
            kind: StorageKind::S3,
            bucket: "team-backup".to_string(),
            access_key_id: FINAL_ID.to_string(),
            secret_access_key: "configured-secret-canary".to_string(),
            account_id: ACCOUNT.to_string(),
            s3_endpoint: format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
            region: "auto".to_string(),
            local_dir: String::new(),
            supports_conditional_writes: Some(true),
        });
        repository.save_config(config).unwrap();

        let setup_id = "r2setup-111111111111111111111111";
        let mut entry = journal_entry(setup_id, R2SetupStage::ProbeRunning);
        entry.final_token_id = Some(FINAL_ID.to_string());
        R2SetupJournalStore::new(&repository).save(&entry).unwrap();
        let api = FakeApi::new(&[]);
        let probe = FakeProbe::successful();

        let result = cleanup_r2_setup_with(
            &repository,
            CleanupR2SetupRequest {
                setup_id: setup_id.to_string(),
                bootstrap_token: SecretString::new(BOOTSTRAP_CANARY.to_string()),
            },
            &api,
            &probe,
            now(),
        )
        .unwrap();

        assert!(result.cleaned);
        assert_eq!(probe.reconcile_calls.get(), 1);
        assert!(api.revoked.borrow().is_empty());
        assert!(R2SetupJournalStore::new(&repository)
            .list()
            .unwrap()
            .is_empty());
        assert_eq!(
            repository.load_config().unwrap().storages[0].access_key_id,
            FINAL_ID
        );
    }
}
