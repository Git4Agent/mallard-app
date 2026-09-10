use mallard_application::storage_setup::{
    begin_storage_draft_operation, list_storages, prepare_storage_draft_in_operation,
    verify_storage, PreparedProviderConfig,
};
use mallard_domain::{BoundedConfigObject, SecretMap, StorageId, StorageSetupDraftId};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::r2::{
    provider_key as r2_provider_key, secret_access_key, R2SettingsV1,
};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;
use serde::Serialize;
use tokio::runtime::{Handle, Runtime};

use crate::error::CliError;
use crate::r2_browser_setup::{authorize_bucket, derive_destination, WranglerClient};

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

pub(crate) trait StorageBackend {
    fn list(&self) -> Result<Vec<StorageSummary>, CliError>;
    fn setup(&self, input: SetupInput) -> Result<SetupOutcome, CliError>;
    fn setup_dropbox(&self, input: SetupInput) -> Result<SetupOutcome, CliError>;
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
    fn setup_dropbox(&self, input: SetupInput) -> Result<SetupOutcome, CliError> {
        let client = mallard_infrastructure::dropbox_auth::DropboxClient::new()
            .map_err(dropbox_auth_error)?;
        prepare_dropbox(
            &self.repository,
            self.registry(),
            &self.runtime,
            input,
            &client,
            crate::dropbox_browser_setup::authorize,
        )
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

    /// The R2 provisioning assistant: pin the exact draft, drive Cloudflare's
    /// Wrangler login and bucket provisioning, mint an exact-bucket
    /// credential, and prepare the ordinary draft for the user's review.
    fn setup(&self, input: SetupInput) -> Result<SetupOutcome, CliError> {
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
        let account_id = wrangler.authenticated_account_id()?;
        let destination = derive_destination(&account_id, &input.name);

        wrangler.ensure_bucket(&destination.account_id, &destination.bucket)?;
        let credentials = authorize_bucket(&destination.account_id, &destination.bucket)?;
        let settings = BoundedConfigObject::encode(&R2SettingsV1 {
            api_url: destination.api_url,
            bucket: destination.bucket,
            account_id: destination.account_id,
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
        Ok(SetupOutcome { reconnected: false })
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

fn dropbox_auth_error(error: mallard_infrastructure::dropbox_auth::AuthError) -> CliError {
    CliError::remote(error.code(), error.message())
}
fn dropbox_setup_error(error: mallard_infrastructure::dropbox_setup::SetupError) -> CliError {
    CliError::remote(error.code(), error.message())
}

fn prepare_dropbox(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    runtime: &Runtime,
    input: SetupInput,
    client: &mallard_infrastructure::dropbox_auth::DropboxClient,
    authorize: impl FnOnce(
        &mallard_infrastructure::dropbox_auth::AuthorizationRequest,
    ) -> Result<zeroize::Zeroizing<String>, CliError>,
) -> Result<SetupOutcome, CliError> {
    use mallard_infrastructure::dropbox_auth::{
        AuthorizationRequest, SETUP_SCOPES, STORAGE_SCOPES,
    };
    use mallard_infrastructure::dropbox_setup::{prepare_namespace, DropboxSetupJournal};
    use mallard_infrastructure::storage_providers::dropbox::{
        provider_key, refresh_token_key, DropboxSettingsV1,
    };

    let draft_id = StorageSetupDraftId::parse(input.draft_id)
        .map_err(|_| CliError::invalid("The Dropbox draft ID is invalid."))?;
    let operation =
        begin_storage_draft_operation(repository, &draft_id, input.expected_revision, &input.name)
            .map_err(CliError::from_setup)?;
    if operation.provider() != &provider_key() {
        return Err(CliError::invalid_code(
            "storage_setup_draft_not_dropbox",
            "The requested draft is not a Dropbox draft.",
        ));
    }
    let draft = repository
        .load_storage_setup_draft(&draft_id)
        .map_err(|_| CliError::persistence("Mallard could not load the Dropbox draft."))?
        .ok_or_else(|| CliError::invalid("The Dropbox draft no longer exists."))?;
    let settings: serde_json::Value = serde_json::to_value(&draft.storage.provider_config.settings)
        .map_err(|_| CliError::persistence("Mallard could not read the Dropbox settings."))?;
    let namespace = settings["namespace_id"]
        .as_str()
        .filter(|value| !value.is_empty());
    // An edit draft regains access to the namespace it already has; the
    // browser consent below asks for the storage scopes only.
    let reconnected = matches!(
        draft.source,
        mallard_domain::StorageSetupDraftSource::Edit { .. }
    );
    if reconnected && namespace.is_none() {
        return Err(CliError::invalid(
            "The existing Dropbox connection has no namespace.",
        ));
    }
    let mut journal =
        DropboxSetupJournal::open(repository, &draft_id).map_err(dropbox_setup_error)?;
    let scopes = if namespace.is_some() {
        STORAGE_SCOPES
    } else {
        SETUP_SCOPES
    };
    let request = AuthorizationRequest::new(scopes).map_err(dropbox_auth_error)?;
    let code = authorize(&request)?;
    let grant = runtime
        .block_on(client.exchange(&code, &request.verifier, scopes))
        .map_err(dropbox_auth_error)?;
    let namespace_id = runtime
        .block_on(prepare_namespace(client, &grant, namespace, &mut journal))
        .map_err(dropbox_setup_error)?;
    let settings = BoundedConfigObject::encode(&DropboxSettingsV1 { namespace_id })
        .map_err(|_| CliError::persistence("Mallard could not encode the Dropbox connection."))?;
    let mut secrets = SecretMap::empty();
    secrets
        .insert(refresh_token_key(), grant.refresh)
        .map_err(|_| CliError::persistence("Mallard could not store the Dropbox credential."))?;
    prepare_storage_draft_in_operation(
        &operation,
        repository,
        registry,
        PreparedProviderConfig { settings, secrets },
    )
    .map_err(CliError::from_setup)?;
    journal.finish().map_err(dropbox_setup_error)?;
    Ok(SetupOutcome { reconnected })
}

#[cfg(test)]
mod dropbox_tests {
    use super::*;
    use mallard_application::storage_setup::{
        create_storage_setup_draft, get_storage_setup_draft, update_storage_setup_draft,
        UpdateStorageSetupDraftRequest,
    };
    use mallard_infrastructure::dropbox_setup::mock::MockDropboxSetup;
    use mallard_infrastructure::storage_providers::dropbox::{provider_key, refresh_token_key};
    use std::collections::BTreeMap;

    #[test]
    fn oauth_fills_exact_draft_and_exposes_refresh_only_for_new_draft_review() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry = StorageProviderRegistry::production();
        let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let mock = MockDropboxSetup::new();
        let input = || SetupInput {
            name: draft.name.clone(),
            draft_id: draft.draft_id.to_string(),
            expected_revision: draft.revision,
        };
        let mut stale = input();
        stale.expected_revision += 1;
        assert!(prepare_dropbox(
            &repository,
            registry,
            &mock.runtime,
            stale,
            &mock.client(),
            |_| panic!("stale draft must not open the browser")
        )
        .is_err());
        assert_eq!(
            prepare_dropbox(
                &repository,
                registry,
                &mock.runtime,
                input(),
                &mock.client(),
                |_| Ok(zeroize::Zeroizing::new("callback-code".into())),
            )
            .unwrap(),
            SetupOutcome { reconnected: false }
        );
        let raw = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(raw.storage.provider_config.version, 1);
        assert_eq!(raw.storage.provider_config.secrets.len(), 1);
        assert!(raw
            .storage
            .provider_config
            .secrets
            .contains(&refresh_token_key()));
        let summary = get_storage_setup_draft(&repository, registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            summary
                .secrets
                .iter()
                .find(|s| s.secret_id == refresh_token_key())
                .unwrap()
                .value
                .as_deref(),
            Some("refresh-canary")
        );
        let listed =
            mallard_application::storage_setup::list_storage_setup_drafts(&repository, registry)
                .unwrap()
                .0;
        assert!(listed
            .iter()
            .all(|draft| draft.secrets.iter().all(|s| s.value.is_none())));
        assert!(repository.load_config().unwrap().storages.is_empty());
        let replace = update_storage_setup_draft(
            &repository,
            registry,
            UpdateStorageSetupDraftRequest {
                draft_id: summary.draft_id.clone(),
                revision: summary.revision,
                name: summary.name.clone(),
                settings: summary.settings.clone(),
                secrets: BTreeMap::from([(
                    mallard_infrastructure::storage_providers::dropbox::token_key(),
                    mallard_domain::SecretOperation::Set {
                        value: mallard_domain::SecretValue::parse("manual-canary").unwrap(),
                    },
                )]),
            },
        );
        assert_eq!(
            replace.unwrap_err().code(),
            "storage_managed_credentials_locked"
        );
        let replaced = update_storage_setup_draft(
            &repository,
            registry,
            UpdateStorageSetupDraftRequest {
                draft_id: summary.draft_id.clone(),
                revision: summary.revision,
                name: summary.name.clone(),
                settings: summary.settings.clone(),
                secrets: BTreeMap::from([(
                    refresh_token_key(),
                    mallard_domain::SecretOperation::Replace {
                        value: mallard_domain::SecretValue::parse("replacement-canary").unwrap(),
                    },
                )]),
            },
        )
        .unwrap();
        assert!(replaced.secrets.iter().all(|s| s.value.is_none()));
        let summary = get_storage_setup_draft(&repository, registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            summary
                .secrets
                .iter()
                .find(|s| s.secret_id == refresh_token_key())
                .unwrap()
                .value
                .as_deref(),
            Some("replacement-canary")
        );
        // Reconnect the prepared namespace; no second folder is created.
        prepare_dropbox(
            &repository,
            registry,
            &mock.runtime,
            SetupInput {
                name: summary.name.clone(),
                draft_id: summary.draft_id.to_string(),
                expected_revision: summary.revision,
            },
            &mock.client(),
            |_| Ok(zeroize::Zeroizing::new("callback-code".into())),
        )
        .unwrap();
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        assert_eq!(mock.count("sharing/share_folder"), 1);
        // Match R2: reopening committed storage reports presence, not values.
        let prepared = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        let mut config = repository.load_config().unwrap();
        config.storages.push(prepared.storage.clone());
        repository.save_config(config).unwrap();
        let edit = mallard_application::storage_setup::create_storage_edit_draft(
            &repository,
            registry,
            &prepared.storage.id,
        )
        .unwrap();
        let edit = get_storage_setup_draft(&repository, registry, &edit.draft_id)
            .unwrap()
            .unwrap();
        assert!(edit.edits_existing);
        assert!(edit
            .secrets
            .iter()
            .any(|s| s.secret_id == refresh_token_key() && s.present));
        assert!(edit.secrets.iter().all(|s| s.value.is_none()));
        // Regaining access for the committed storage reports reconnection,
        // keeps the namespace, and creates nothing.
        assert_eq!(
            prepare_dropbox(
                &repository,
                registry,
                &mock.runtime,
                SetupInput {
                    name: edit.name.clone(),
                    draft_id: edit.draft_id.to_string(),
                    expected_revision: edit.revision,
                },
                &mock.client(),
                |_| Ok(zeroize::Zeroizing::new("callback-code".into())),
            )
            .unwrap(),
            SetupOutcome { reconnected: true }
        );
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        assert_eq!(mock.count("sharing/share_folder"), 1);
        let reconnected = repository
            .load_storage_setup_draft(&edit.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            reconnected.storage.provider_config.settings,
            prepared.storage.provider_config.settings
        );
        assert!(reconnected
            .storage
            .provider_config
            .secrets
            .contains(&refresh_token_key()));
    }

    #[test]
    fn concurrent_edit_is_preserved_and_same_folder_can_be_resumed() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry = StorageProviderRegistry::production();
        let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let mock = MockDropboxSetup::new();
        let result = prepare_dropbox(
            &repository,
            registry,
            &mock.runtime,
            SetupInput {
                name: draft.name.clone(),
                draft_id: draft.draft_id.to_string(),
                expected_revision: draft.revision,
            },
            &mock.client(),
            |_| {
                update_storage_setup_draft(
                    &repository,
                    registry,
                    UpdateStorageSetupDraftRequest {
                        draft_id: draft.draft_id.clone(),
                        revision: draft.revision,
                        name: "Changed by user".into(),
                        settings: BoundedConfigObject::empty(),
                        secrets: BTreeMap::new(),
                    },
                )
                .unwrap();
                Ok(zeroize::Zeroizing::new("callback-code".into()))
            },
        );
        assert_eq!(
            result.unwrap_err().code(),
            "storage_setup_draft_revision_conflict"
        );
        let changed = get_storage_setup_draft(&repository, registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(changed.name, "Changed by user");
        assert!(changed.secrets.iter().all(|s| !s.present));
        prepare_dropbox(
            &repository,
            registry,
            &mock.runtime,
            SetupInput {
                name: changed.name.clone(),
                draft_id: changed.draft_id.to_string(),
                expected_revision: changed.revision,
            },
            &mock.client(),
            |_| Ok(zeroize::Zeroizing::new("callback-code".into())),
        )
        .unwrap();
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        assert!(repository.load_config().unwrap().storages.is_empty());
    }
}
