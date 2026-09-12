use mallard_application::storage_setup::{
    begin_storage_draft_operation, list_storages, prepare_storage_draft_in_operation,
    verify_storage, PreparedProviderConfig,
};
use mallard_domain::{BoundedConfigObject, SecretMap, StorageId, StorageSetupDraftId};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::r2_setup::{
    prepare_bucket, propose_bucket_name, R2SetupClient, R2SetupJournal,
};
use mallard_infrastructure::storage_providers::r2::{
    parse_r2_api_url, provider_key as r2_provider_key, secret_access_key, R2Jurisdiction,
    R2SettingsV1,
};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;
use serde::Serialize;
use tokio::runtime::{Handle, Runtime};

use crate::error::CliError;
use crate::r2_browser_setup::{
    authorize_bucket, publish_bucket_ready, publish_wrangler_ready, WranglerAccount, WranglerClient,
};

#[derive(Serialize)]
pub(crate) struct StorageSummary {
    pub storage_id: String,
    pub name: String,
    pub provider: String,
    pub provider_label: String,
}

pub(crate) struct SetupInput {
    pub name: String,
    pub draft_id: String,
    pub expected_revision: u64,
}

pub(crate) struct VerifyOutcome {
    pub storage_id: String,
    pub config_revision: u64,
    pub verified: bool,
}

/// What a setup command prepared: a draft for a new storage, or a regained
/// credential for a storage that already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SetupOutcome {
    pub reconnected: bool,
}

pub(crate) struct ExistingBucket {
    pub name: String,
    pub creation_date: String,
}

/// Which bucket a setup is for: a new one, named by the caller or by the
/// helper, or an existing one the scan listed.
pub(crate) enum R2Target {
    New { bucket: Option<String> },
    Existing(ExistingBucket),
}

pub(crate) trait StorageBackend {
    fn list(&self) -> Result<Vec<StorageSummary>, CliError>;
    fn r2_accounts(&self, login: bool) -> Result<Vec<WranglerAccount>, CliError>;
    fn setup(
        &self,
        input: SetupInput,
        login: bool,
        account_id: Option<String>,
        jurisdiction: R2Jurisdiction,
        target: R2Target,
    ) -> Result<SetupOutcome, CliError>;
    fn discover_r2(
        &self,
        account_id: &str,
    ) -> Result<mallard_infrastructure::r2_setup::discovery::BucketScan, CliError>;
    fn verify(&self, storage_id: &str) -> Result<VerifyOutcome, CliError>;
}

pub(crate) struct ProductionBackend {
    repository: LocalStateRepository,
    runtime: Runtime,
}

impl ProductionBackend {
    pub fn from_user_home() -> Result<Self, CliError> {
        let repository = LocalStateRepository::from_user_home().map_err(|_| {
            CliError::persistence(
                "Mallard could not access its local storage configuration. If this command is running in a restricted environment, approve read/write access to Mallard's local state and run the same command again.",
            )
        })?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| CliError::persistence("Mallard could not start its storage worker."))?;
        Ok(Self {
            repository,
            runtime,
        })
    }

    fn runtime_handle(&self) -> Handle {
        self.runtime.handle().clone()
    }

    fn registry(&self) -> &'static StorageProviderRegistry {
        StorageProviderRegistry::production()
    }
}

impl StorageBackend for ProductionBackend {
    /// The first step of every desktop setup: which Wrangler this is, said
    /// as soon as it passes, then the session's accounts — after a browser
    /// login when there is no session or a new one was asked for.
    fn r2_accounts(&self, login: bool) -> Result<Vec<WranglerAccount>, CliError> {
        let wrangler = WranglerClient::new()?;
        let info = wrangler.check_dependency()?;
        publish_wrangler_ready(&info)
            .map_err(|_| CliError::persistence("Mallard could not report setup progress."))?;
        if login {
            wrangler.login()?;
        }
        wrangler.authenticated_accounts()
    }

    fn discover_r2(
        &self,
        account_id: &str,
    ) -> Result<mallard_infrastructure::r2_setup::discovery::BucketScan, CliError> {
        let wrangler = WranglerClient::new()?;
        wrangler.check_dependency()?;
        let account_id = wrangler.selected_account_id(Some(account_id))?;
        let client = R2SetupClient::new(wrangler.oauth_token()?).map_err(r2_setup_error)?;
        self.runtime
            .block_on(client.discover(&account_id))
            .map_err(r2_setup_error)
    }

    fn list(&self) -> Result<Vec<StorageSummary>, CliError> {
        Ok(list_storages(&self.repository, self.registry())
            .map_err(CliError::from_setup)?
            .into_iter()
            .map(|storage| StorageSummary {
                storage_id: storage.storage_id.to_string(),
                name: storage.name,
                provider: storage.provider.to_string(),
                provider_label: storage.provider_label,
            })
            .collect())
    }

    /// The R2 setup operation: pin the exact draft, drive Cloudflare's
    /// Wrangler login and bucket provisioning, mint an exact-bucket
    /// credential, and prepare the ordinary draft for the user's review.
    fn setup(
        &self,
        input: SetupInput,
        login: bool,
        account_id: Option<String>,
        jurisdiction: R2Jurisdiction,
        target: R2Target,
    ) -> Result<SetupOutcome, CliError> {
        let draft_id = StorageSetupDraftId::parse(input.draft_id).map_err(|_| {
            CliError::invalid_code(
                "storage_setup_draft_id_invalid",
                "The Mallard storage setup draft ID is invalid.",
            )
        })?;
        let operation = begin_storage_draft_operation(
            &self.repository,
            &draft_id,
            input.expected_revision,
            &input.name,
        )
        .map_err(CliError::from_setup)?;
        if operation.provider() != &r2_provider_key() {
            return Err(CliError::invalid_code(
                "storage_setup_draft_not_r2",
                "The requested storage setup draft is not an R2 draft.",
            ));
        }
        let wrangler = WranglerClient::new()?;
        wrangler.check_dependency()?;
        if login {
            wrangler.login()?;
        }
        let account_id = wrangler.selected_account_id(account_id.as_deref())?;
        let draft = self
            .repository
            .load_storage_setup_draft(&draft_id)
            .map_err(|_| CliError::persistence("Mallard could not load the R2 draft."))?
            .ok_or_else(|| CliError::invalid("The R2 draft no longer exists."))?;
        let settings: serde_json::Value =
            serde_json::to_value(&draft.storage.provider_config.settings)
                .map_err(|_| CliError::persistence("Mallard could not read the R2 settings."))?;
        let existing = settings["api_url"]
            .as_str()
            .filter(|value| !value.is_empty());
        let reconnected = matches!(
            draft.source,
            mallard_domain::StorageSetupDraftSource::Edit { .. }
        );
        let selected = match &target {
            R2Target::Existing(bucket) => Some(bucket),
            R2Target::New { .. } => None,
        };
        let mut journal = if selected.is_none() {
            Some(
                R2SetupJournal::open(&self.repository, &draft_id, &account_id, jurisdiction)
                    .map_err(r2_setup_error)?,
            )
        } else {
            None
        };
        let selected_client = if selected.is_some() {
            Some(R2SetupClient::new(wrangler.oauth_token()?).map_err(r2_setup_error)?)
        } else {
            None
        };
        let destination = if let Some(bucket) = &selected {
            let destination = self
                .runtime
                .block_on(selected_client.as_ref().unwrap().verify_existing(
                    &account_id,
                    jurisdiction,
                    &bucket.name,
                    &bucket.creation_date,
                ))
                .map_err(r2_setup_error)?;
            if reconnected || existing.is_some_and(|url| url != destination.api_url()) {
                return Err(r2_setup_error(
                    mallard_infrastructure::r2_setup::SetupError::Identity,
                ));
            }
            let config = self
                .repository
                .load_config()
                .map_err(|_| CliError::persistence("Mallard could not read configured storage."))?;
            if config.storages.iter().any(|storage| {
                storage.provider == r2_provider_key()
                    && storage.provider_config.settings.get_str("api_url")
                        == Some(destination.api_url())
            }) {
                return Err(CliError::invalid_code(
                    "r2_bucket_already_connected",
                    "This bucket is already connected.",
                ));
            }
            destination
        } else if let Some(api_url) = existing {
            let destination = parse_r2_api_url(api_url)
                .map_err(|_| CliError::invalid("The saved R2 destination is invalid."))?;
            if destination.account_id() != account_id {
                return Err(r2_setup_error(
                    mallard_infrastructure::r2_setup::SetupError::Identity,
                ));
            }
            destination
        } else {
            if reconnected {
                return Err(CliError::invalid(
                    "The existing R2 connection has no destination.",
                ));
            }
            let client = R2SetupClient::new(wrangler.oauth_token()?).map_err(r2_setup_error)?;
            let requested = match &target {
                R2Target::New {
                    bucket: Some(bucket),
                } => bucket.clone(),
                _ => propose_bucket_name().map_err(r2_setup_error)?,
            };
            let destination = self
                .runtime
                .block_on(prepare_bucket(
                    &client,
                    journal.as_mut().unwrap(),
                    &requested,
                ))
                .map_err(r2_setup_error)?;
            publish_bucket_ready(destination.bucket())
                .map_err(|_| CliError::persistence("Mallard could not report setup progress."))?;
            destination
        };
        let credentials = authorize_bucket(destination.account_id(), destination.bucket())?;
        if let Some(bucket) = &selected {
            self.runtime
                .block_on(selected_client.as_ref().unwrap().verify_existing(
                    &account_id,
                    jurisdiction,
                    &bucket.name,
                    &bucket.creation_date,
                ))
                .map_err(r2_setup_error)?;
        }
        let settings = BoundedConfigObject::encode(&R2SettingsV1 {
            api_url: destination.api_url().into(),
            bucket: destination.bucket().into(),
            account_id: destination.account_id().into(),
            access_key_id: credentials.access_key_id(),
        })
        .map_err(|_| CliError::persistence("Mallard could not encode the R2 settings."))?;
        let mut secrets = SecretMap::empty();
        secrets
            .insert(
                secret_access_key(),
                credentials
                    .secret_access_key()
                    .into_secret_value()
                    .map_err(CliError::from_setup)?,
            )
            .map_err(|_| CliError::persistence("Mallard could not store the R2 secret."))?;
        prepare_storage_draft_in_operation(
            &operation,
            &self.repository,
            self.registry(),
            PreparedProviderConfig { settings, secrets },
        )
        .map_err(CliError::from_setup)?;
        if let Some(journal) = journal {
            journal.finish().map_err(r2_setup_error)?;
        }
        Ok(SetupOutcome { reconnected })
    }

    fn verify(&self, storage_id: &str) -> Result<VerifyOutcome, CliError> {
        let storage_id = StorageId::parse(storage_id)
            .map_err(|_| CliError::invalid("The Mallard storage ID is invalid."))?;
        let result = verify_storage(
            &self.repository,
            self.registry(),
            &self.runtime_handle(),
            &storage_id,
        )
        .map_err(CliError::from_setup)?;
        Ok(VerifyOutcome {
            storage_id: result.storage_id.to_string(),
            config_revision: result.config_revision,
            verified: result.verified,
        })
    }
}

fn r2_setup_error(error: mallard_infrastructure::r2_setup::SetupError) -> CliError {
    CliError::remote(error.code(), error.message())
}
