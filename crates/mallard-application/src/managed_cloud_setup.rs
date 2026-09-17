//! Shared Mallard Cloud draft preparation for the desktop setup flow: the
//! signed-in account and its token become the draft's settings and secret.
use crate::storage_setup::{
    prepare_storage_draft_in_operation, PrepareStorageDraftResult, PreparedProviderConfig,
    StorageDraftOperation, StorageSetupError,
};
use mallard_domain::{
    BoundedConfigObject, SecretMap, SecretValue, StorageSetupDraftId, StorageSetupDraftSource,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::{
    managed_cloud::{provider_key, token_key, ManagedCloudSettingsV1},
    StorageProviderRegistry,
};

/// The account a sign-in produced.
pub struct SignedInAccount<'a> {
    pub service_origin: &'a str,
    pub account_id: &'a str,
    pub account_login: &'a str,
    pub token: &'a SecretValue,
}

pub fn prepare(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    operation: &StorageDraftOperation,
    draft_id: &StorageSetupDraftId,
    account: SignedInAccount<'_>,
) -> Result<PrepareStorageDraftResult, StorageSetupError> {
    let mismatch = || {
        StorageSetupError::invalid(
            "managed_cloud_account_mismatch",
            "The sign-in belongs to another Mallard Cloud account than this storage.",
        )
    };
    if operation.draft_id() != draft_id || operation.provider() != &provider_key() {
        return Err(mismatch());
    }
    let draft = repository
        .load_storage_setup_draft(draft_id)
        .map_err(|_| StorageSetupError::persistence())?
        .ok_or_else(mismatch)?;
    let saved: Option<ManagedCloudSettingsV1> =
        draft.storage.provider_config.settings.decode().ok();
    // A reconnect keeps the storage's account; a new setup takes whatever
    // account signed in.
    if matches!(draft.source, StorageSetupDraftSource::Edit { .. }) {
        match &saved {
            Some(saved)
                if saved.service_origin == account.service_origin
                    && saved.account_id == account.account_id => {}
            _ => return Err(mismatch()),
        }
    }
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    if config.storages.iter().any(|storage| {
        storage.id != draft.storage.id
            && storage.provider == provider_key()
            && storage.provider_config.settings.get_str("service_origin")
                == Some(account.service_origin)
            && storage.provider_config.settings.get_str("account_id") == Some(account.account_id)
    }) {
        return Err(StorageSetupError::invalid(
            "storage_managed_account_exists",
            "This Mallard Cloud account is already connected as a storage.",
        ));
    }
    let settings = BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
        service_origin: account.service_origin.to_owned(),
        account_id: account.account_id.to_owned(),
        account_login: account.account_login.to_owned(),
    })
    .map_err(|_| StorageSetupError::persistence())?;
    let mut secrets = SecretMap::empty();
    secrets
        .insert(token_key(), account.token.clone())
        .map_err(|_| StorageSetupError::persistence())?;
    prepare_storage_draft_in_operation(
        operation,
        repository,
        registry,
        PreparedProviderConfig { settings, secrets },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_setup::{
        begin_storage_draft_operation, create_storage_setup_draft, get_storage_setup_draft,
    };
    use mallard_infrastructure::managed_cloud::PRODUCTION_ORIGIN;

    const TOKEN: &str = "desktop-session-token-0123456789";

    #[test]
    fn prepare_pins_the_account_refuses_duplicates_and_keeps_the_token_out_of_listings() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry = StorageProviderRegistry::production();
        let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let operation = begin_storage_draft_operation(
            &repository,
            &draft.draft_id,
            draft.revision,
            &draft.name,
        )
        .unwrap();
        let token = SecretValue::parse(TOKEN).unwrap();
        let account = |login: &'static str| SignedInAccount {
            service_origin: PRODUCTION_ORIGIN,
            account_id: "user_1",
            account_login: login,
            token: &token,
        };
        prepare(
            &repository,
            registry,
            &operation,
            &draft.draft_id,
            account("octocat"),
        )
        .unwrap();
        drop(operation);
        let prepared = get_storage_setup_draft(&repository, registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.revision, draft.revision + 1);
        assert_eq!(prepared.settings.get_str("account_login"), Some("octocat"));
        assert_eq!(
            prepared.settings.get_str("service_origin"),
            Some(PRODUCTION_ORIGIN)
        );
        assert!(prepared
            .secrets
            .iter()
            .any(|secret| secret.secret_id == token_key() && secret.present));
        assert!(!serde_json::to_string(&prepared.settings)
            .unwrap()
            .contains(TOKEN));
        // Once the account is a saved storage, a second draft for the same
        // account is refused before any write. Saved directly: a commit
        // would probe the service.
        let mut config = repository.load_config().unwrap();
        let draft_config = repository
            .load_storage_setup_draft(&draft.draft_id)
            .unwrap()
            .unwrap();
        config.storages.push(draft_config.storage.clone());
        repository.save_config(config).unwrap();
        assert_eq!(repository.load_config().unwrap().storages.len(), 1);
        let again = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let operation = begin_storage_draft_operation(
            &repository,
            &again.draft_id,
            again.revision,
            &again.name,
        )
        .unwrap();
        assert_eq!(
            prepare(
                &repository,
                registry,
                &operation,
                &again.draft_id,
                account("octocat")
            )
            .unwrap_err()
            .code(),
            "storage_managed_account_exists"
        );
    }
}
