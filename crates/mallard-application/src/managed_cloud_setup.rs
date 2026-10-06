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
    pub github_user_id: &'a str,
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
                    && saved.github_user_id == account.github_user_id => {}
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
            && storage.provider_config.settings.get_str("github_user_id")
                == Some(account.github_user_id)
    }) {
        return Err(StorageSetupError::invalid(
            "storage_managed_account_exists",
            "This Mallard Cloud account is already connected as a storage.",
        ));
    }
    let settings = BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
        service_origin: account.service_origin.to_owned(),
        github_user_id: account.github_user_id.to_owned(),
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
        // Use only the adapter under test.
        let registry = &StorageProviderRegistry::from_adapters(vec![std::sync::Arc::new(
            mallard_infrastructure::storage_providers::managed_cloud::ManagedCloudAdapter,
        )])
        .unwrap();
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
            github_user_id: "123",
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

    #[test]
    fn reconnect_uses_github_identity_and_commit_refuses_duplicate_drafts() {
        use crate::storage_setup::{commit_storage_setup_draft, create_storage_edit_draft};
        use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
        use mallard_infrastructure::storage_providers::managed_cloud::ManagedCloudAdapter;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let stub = runtime.block_on(StubManagedCloud::start());
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry =
            StorageProviderRegistry::from_adapters(vec![std::sync::Arc::new(ManagedCloudAdapter)])
                .unwrap();
        let (user, token) = stub.state.lock().unwrap().add_user("octocat");
        let token = SecretValue::parse(token).unwrap();
        let prepare_account = |draft: &crate::storage_setup::StorageSetupDraftSummary,
                               github_user_id: &str,
                               login: &str,
                               token: &SecretValue| {
            let operation = begin_storage_draft_operation(
                &repository,
                &draft.draft_id,
                draft.revision,
                &draft.name,
            )
            .unwrap();
            prepare(
                &repository,
                &registry,
                &operation,
                &draft.draft_id,
                SignedInAccount {
                    service_origin: &stub.endpoint,
                    github_user_id,
                    account_login: login,
                    token,
                },
            )
        };
        // Both drafts pass preparation before either is committed.
        let first = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        let second = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        let first_ready = prepare_account(&first, &user.github_user_id, "octocat", &token).unwrap();
        let second_ready =
            prepare_account(&second, &user.github_user_id, "renamed", &token).unwrap();
        let commit = |id: &StorageSetupDraftId, revision| {
            commit_storage_setup_draft(
                &repository,
                &registry,
                runtime.handle(),
                id,
                revision,
                |_| Ok(()),
            )
        };
        let saved = commit(&first.draft_id, first_ready.draft_revision).unwrap();
        let before = repository.load_config().unwrap();
        assert_eq!(
            commit(&second.draft_id, second_ready.draft_revision)
                .unwrap_err()
                .code(),
            "storage_destination_conflict"
        );
        assert_eq!(repository.load_config().unwrap(), before);

        // Cloud recreated its internal user ID. GitHub identity is unchanged.
        let (replacement, replacement_token) = {
            let mut state = stub.state.lock().unwrap();
            state.revoke_token(token.expose());
            state.users.remove(&user.id);
            let (mut replacement, token) = state.add_user("renamed");
            replacement.github_user_id = user.github_user_id.clone();
            state
                .users
                .insert(replacement.id.clone(), replacement.clone());
            (replacement, token)
        };
        assert_ne!(replacement.id, user.id);
        let replacement_token = SecretValue::parse(replacement_token).unwrap();
        let edit = create_storage_edit_draft(&repository, &registry, &saved.storage_id).unwrap();
        let untouched_draft = repository.load_storage_setup_draft(&edit.draft_id).unwrap();
        assert_eq!(
            prepare_account(&edit, "999999", "someone-else", &replacement_token)
                .unwrap_err()
                .code(),
            "managed_cloud_account_mismatch"
        );
        assert_eq!(
            repository.load_storage_setup_draft(&edit.draft_id).unwrap(),
            untouched_draft
        );
        assert_eq!(repository.load_config().unwrap(), before);
        // A refused replacement token must not change the committed storage.
        let refused = prepare_account(&edit, &user.github_user_id, "renamed", &token).unwrap();
        assert_eq!(
            commit(&edit.draft_id, refused.draft_revision)
                .unwrap_err()
                .code(),
            "managed_cloud_reconnect_required"
        );
        assert_eq!(repository.load_config().unwrap(), before);
        let edit = get_storage_setup_draft(&repository, &registry, &edit.draft_id)
            .unwrap()
            .unwrap();
        let ready =
            prepare_account(&edit, &user.github_user_id, "renamed", &replacement_token).unwrap();
        let reconnected = commit(&edit.draft_id, ready.draft_revision).unwrap();
        assert!(reconnected.replaced);
        assert_eq!(reconnected.storage_id, saved.storage_id);
        let after = repository.load_config().unwrap();
        assert_eq!(after.storages.len(), 1);
        assert_eq!(after.links, before.links);
        assert_eq!(after.storages[0].name, before.storages[0].name);
        assert_eq!(after.storages[0].provider_config.version, 1);
        let settings = &after.storages[0].provider_config.settings;
        assert_eq!(
            settings.get_str("github_user_id"),
            Some(user.github_user_id.as_str())
        );
        assert_eq!(settings.get_str("account_login"), Some("renamed"));
        assert_eq!(settings.get_str("account_id"), None);
    }
}
