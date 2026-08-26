use mallard_application::storage_setup::{
    begin_r2_storage_draft_operation, list_storages, prepare_r2_storage_draft_in_operation,
    verify_storage, PrepareR2StorageDraftRequest,
};
use mallard_domain::{StorageId, StorageKind, StorageSetupDraftId};
use mallard_infrastructure::persistence::LocalStateRepository;
use serde::Serialize;
use tokio::runtime::{Handle, Runtime};

use crate::error::CliError;
use crate::r2_browser_setup::{authorize_bucket, derive_destination, WranglerClient};

#[derive(Serialize)]
pub(crate) struct StorageSummary {
    pub storage_id: String,
    pub name: String,
    pub kind: &'static str,
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

pub(crate) trait StorageBackend {
    fn list(&self) -> Result<Vec<StorageSummary>, CliError>;
    fn setup(&self, input: SetupInput) -> Result<(), CliError>;
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

    fn setup(&self, input: SetupInput) -> Result<(), CliError> {
        let draft_id = StorageSetupDraftId::parse(input.draft_id).map_err(|_| {
            CliError::invalid_code(
                "storage_setup_draft_id_invalid",
                "The Mallard storage setup draft ID is invalid.",
            )
        })?;
        let operation = begin_r2_storage_draft_operation(
            &self.repository,
            &draft_id,
            input.expected_revision,
            &input.name,
        )
        .map_err(CliError::from_setup)?;
        let wrangler = WranglerClient::new()?;
        let account_id = wrangler.authenticated_account_id()?;
        let destination = derive_destination(&account_id, &input.name);

        wrangler.ensure_bucket(&destination.account_id, &destination.bucket)?;
        let credentials = authorize_bucket(&destination.account_id, &destination.bucket)?;
        prepare_r2_storage_draft_in_operation(
            &operation,
            &self.repository,
            PrepareR2StorageDraftRequest {
                api_url: destination.api_url,
                access_key_id: credentials.access_key_id(),
                secret_access_key: credentials.secret_access_key(),
            },
        )
        .map_err(CliError::from_setup)?;
        Ok(())
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
