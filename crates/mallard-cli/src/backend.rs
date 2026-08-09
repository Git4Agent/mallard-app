use mallard_application::cloudflare_r2_setup::{
    cleanup_r2_setup, provision_r2_storage, CleanupR2SetupRequest, ProvisionR2StorageRequest,
};
use mallard_application::storage_setup::{
    connect_r2_storage, list_storages, verify_storage, ConnectR2StorageRequest, R2Jurisdiction,
    SecretString,
};
use mallard_domain::{StorageId, StorageKind};
use mallard_infrastructure::persistence::LocalStateRepository;
use serde::Serialize;
use tokio::runtime::{Handle, Runtime};

use crate::error::CliError;

#[derive(Serialize)]
pub(crate) struct StorageSummary {
    pub storage_id: String,
    pub name: String,
    pub kind: &'static str,
}

pub(crate) struct ConnectInput {
    pub name: String,
    pub api_url: String,
    pub access_key_id: String,
    pub secret_access_key: SecretString,
}

pub(crate) struct ConnectOutcome {
    pub storage_id: String,
    pub config_revision: u64,
    pub verified: bool,
}

pub(crate) struct VerifyOutcome {
    pub storage_id: String,
    pub config_revision: u64,
    pub verified: bool,
}

pub(crate) struct ProvisionInput {
    pub name: String,
    pub account_id: String,
    pub bucket: String,
    pub jurisdiction: R2Jurisdiction,
    pub bootstrap_token: SecretString,
}

pub(crate) struct ProvisionOutcome {
    pub storage_id: String,
    pub config_revision: u64,
    pub verified: bool,
    pub bucket_created: bool,
}

pub(crate) struct CleanupInput {
    pub setup_id: String,
    pub bootstrap_token: SecretString,
}

pub(crate) struct CleanupOutcome {
    pub setup_id: String,
    pub cleaned: bool,
}

pub(crate) trait StorageBackend {
    fn list(&self) -> Result<Vec<StorageSummary>, CliError>;
    fn connect(&self, input: ConnectInput) -> Result<ConnectOutcome, CliError>;
    fn provision(&self, input: ProvisionInput) -> Result<ProvisionOutcome, CliError>;
    fn cleanup(&self, input: CleanupInput) -> Result<CleanupOutcome, CliError>;
    fn verify(&self, storage_id: &str) -> Result<VerifyOutcome, CliError>;
}

pub(crate) struct ProductionBackend {
    repository: LocalStateRepository,
    runtime: Runtime,
}

impl ProductionBackend {
    pub fn from_user_home() -> Result<Self, CliError> {
        let repository = LocalStateRepository::from_user_home().map_err(|_| {
            CliError::persistence("Mallard local storage configuration is unavailable.")
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
}

impl StorageBackend for ProductionBackend {
    fn list(&self) -> Result<Vec<StorageSummary>, CliError> {
        Ok(list_storages(&self.repository)
            .map_err(CliError::from_setup)?
            .into_iter()
            .map(|storage| StorageSummary {
                storage_id: storage.storage_id.to_string(),
                name: storage.name,
                kind: match storage.kind {
                    StorageKind::S3 => "r2",
                    StorageKind::Local => "local",
                },
            })
            .collect())
    }

    fn connect(&self, input: ConnectInput) -> Result<ConnectOutcome, CliError> {
        let result = connect_r2_storage(
            &self.repository,
            self.runtime_handle(),
            ConnectR2StorageRequest {
                name: input.name,
                api_url: input.api_url,
                access_key_id: input.access_key_id,
                secret_access_key: input.secret_access_key,
                expected_config_revision: None,
            },
        )
        .map_err(CliError::from_setup)?;
        Ok(ConnectOutcome {
            storage_id: result.storage_id.to_string(),
            config_revision: result.config_revision,
            verified: result.verified,
        })
    }

    fn provision(&self, input: ProvisionInput) -> Result<ProvisionOutcome, CliError> {
        let result = provision_r2_storage(
            &self.repository,
            self.runtime_handle(),
            ProvisionR2StorageRequest {
                name: input.name,
                account_id: input.account_id,
                bucket: input.bucket,
                jurisdiction: input.jurisdiction,
                bootstrap_token: input.bootstrap_token,
                expected_config_revision: None,
            },
        )
        .map_err(CliError::from_setup)?;
        Ok(ProvisionOutcome {
            storage_id: result.storage_id.to_string(),
            config_revision: result.config_revision,
            verified: result.verified,
            bucket_created: result.bucket_created,
        })
    }

    fn cleanup(&self, input: CleanupInput) -> Result<CleanupOutcome, CliError> {
        let result = cleanup_r2_setup(
            &self.repository,
            self.runtime_handle(),
            CleanupR2SetupRequest {
                setup_id: input.setup_id,
                bootstrap_token: input.bootstrap_token,
            },
        )
        .map_err(CliError::from_setup)?;
        Ok(CleanupOutcome {
            setup_id: result.setup_id,
            cleaned: result.cleaned,
        })
    }

    fn verify(&self, storage_id: &str) -> Result<VerifyOutcome, CliError> {
        let storage_id = StorageId::parse(storage_id)
            .map_err(|_| CliError::invalid("The Mallard storage ID is invalid."))?;
        let result = verify_storage(&self.repository, self.runtime_handle(), &storage_id)
            .map_err(CliError::from_setup)?;
        Ok(VerifyOutcome {
            storage_id: result.storage_id.to_string(),
            config_revision: result.config_revision,
            verified: result.verified,
        })
    }
}
