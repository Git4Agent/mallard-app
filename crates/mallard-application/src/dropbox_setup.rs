//! Shared Dropbox draft preparation for the desktop setup flow.
use crate::storage_setup::{
    prepare_storage_draft_in_operation, PrepareStorageDraftResult, PreparedProviderConfig,
    StorageDraftOperation, StorageSetupError,
};
use mallard_domain::{
    BoundedConfigObject, SecretMap, StorageSetupDraftId, StorageSetupDraftSource,
};
use mallard_infrastructure::dropbox_auth::{AuthorizationGrant, DropboxClient};
use mallard_infrastructure::dropbox_setup::{prepare_namespace, DropboxSetupJournal};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::{
    dropbox::{provider_key, refresh_token_key, DropboxSettingsV1},
    StorageProviderRegistry,
};

pub async fn prepare(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    operation: &StorageDraftOperation,
    draft_id: &StorageSetupDraftId,
    client: &DropboxClient,
    grant: &AuthorizationGrant,
    selected_namespace: Option<&str>,
    // The name of the folder a discovery pick chose, for the listing.
    selected_name: Option<&str>,
    folder_name: Option<&str>,
) -> Result<PrepareStorageDraftResult, StorageSetupError> {
    let invalid = || {
        StorageSetupError::invalid(
            "dropbox_setup_identity_mismatch",
            "The Dropbox account or folder does not match this setup.",
        )
    };
    if operation.draft_id() != draft_id || operation.provider() != &provider_key() {
        return Err(invalid());
    }
    let draft = repository
        .load_storage_setup_draft(draft_id)
        .map_err(|_| StorageSetupError::persistence())?
        .ok_or_else(invalid)?;
    let saved = draft
        .storage
        .provider_config
        .settings
        .get_str("namespace_id")
        .filter(|value| !value.is_empty());
    if matches!(draft.source, StorageSetupDraftSource::Edit { .. }) && saved.is_none()
        || saved
            .zip(selected_namespace)
            .is_some_and(|(saved, selected)| saved != selected)
        || selected_namespace.is_some() && folder_name.is_some()
    {
        return Err(invalid());
    }
    let target = saved.or(selected_namespace);
    if let Some(namespace) = target {
        let config = repository
            .load_config()
            .map_err(|_| StorageSetupError::persistence())?;
        if config.storages.iter().any(|storage| {
            storage.id != draft.storage.id
                && storage.provider == provider_key()
                && storage.provider_config.settings.get_str("namespace_id") == Some(namespace)
        }) {
            return Err(StorageSetupError::invalid(
                "dropbox_already_connected",
                "This Dropbox folder is already connected.",
            ));
        }
    }
    let remote = |error: mallard_infrastructure::dropbox_setup::SetupError| {
        StorageSetupError::fixed(
            crate::storage_setup::StorageSetupErrorKind::RemoteFailure,
            error.code(),
            error.message(),
        )
    };
    let mut journal = DropboxSetupJournal::open(repository, draft_id).map_err(remote)?;
    /* What listings call the folder: the name just created, the pick's name,
       or — on a reconnect — whatever the saved settings already say. */
    let saved_name = draft
        .storage
        .provider_config
        .settings
        .get_str("folder_name")
        .map(str::to_string);
    let namespace_id = prepare_namespace(client, grant, target, &mut journal, folder_name)
        .await
        .map_err(remote)?;
    let settings = BoundedConfigObject::encode(&DropboxSettingsV1 {
        namespace_id,
        folder_name: folder_name
            .or(selected_name)
            .map(str::to_string)
            .or(saved_name),
    })
    .map_err(|_| StorageSetupError::persistence())?;
    let mut secrets = SecretMap::empty();
    secrets
        .insert(refresh_token_key(), grant.refresh.clone())
        .map_err(|_| StorageSetupError::persistence())?;
    let result = prepare_storage_draft_in_operation(
        operation,
        repository,
        registry,
        PreparedProviderConfig { settings, secrets },
    )?;
    journal.finish().map_err(remote)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_setup::{
        begin_storage_draft_operation, create_storage_setup_draft, get_storage_setup_draft,
    };
    use mallard_infrastructure::dropbox_auth::{AuthorizationRequest, SETUP_SCOPES};
    use mallard_infrastructure::dropbox_setup::mock::MockDropboxSetup;

    #[test]
    fn direct_setup_prepares_only_the_exact_draft_and_stops_on_rejected_refresh() {
        for rejected in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
            let registry = StorageProviderRegistry::production();
            let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
            let other = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
            let mock = MockDropboxSetup::new();
            let client = mock.client();
            let request = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
            let grant = mock
                .runtime
                .block_on(client.exchange("callback-code", &request.verifier, SETUP_SCOPES))
                .unwrap();
            assert!(begin_storage_draft_operation(
                &repository,
                &draft.draft_id,
                draft.revision + 1,
                &draft.name
            )
            .is_err());
            let operation = begin_storage_draft_operation(
                &repository,
                &draft.draft_id,
                draft.revision,
                &draft.name,
            )
            .unwrap();
            mock.state.lock().unwrap().deny_refresh = rejected;
            let result = mock.runtime.block_on(prepare(
                &repository,
                registry,
                &operation,
                &draft.draft_id,
                &client,
                &grant,
                None,
                None,
                Some("My chosen folder"),
            ));
            if rejected {
                assert_eq!(result.unwrap_err().code(), "dropbox_reconnect_required");
            } else {
                result.unwrap();
            }
            let prepared = get_storage_setup_draft(&repository, registry, &draft.draft_id)
                .unwrap()
                .unwrap();
            assert_eq!(prepared.revision, draft.revision + u64::from(!rejected));
            if !rejected {
                assert_eq!(prepared.settings.get_str("folder_name"), Some("My chosen folder"));
            }
            assert_eq!(
                prepared
                    .secrets
                    .iter()
                    .any(|secret| secret.secret_id == refresh_token_key() && secret.present),
                !rejected
            );
            assert_eq!(
                get_storage_setup_draft(&repository, registry, &other.draft_id)
                    .unwrap()
                    .unwrap()
                    .revision,
                other.revision
            );
            assert!(repository.load_config().unwrap().storages.is_empty());
            assert_eq!(
                mock.state.lock().unwrap().create_paths,
                vec!["/My chosen folder"]
            );
            assert_eq!(mock.count("files/upload"), 0);
        }
    }
    #[test]
    fn existing_namespace_never_creates_or_shares_a_folder() {
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
        let mock = MockDropboxSetup::new();
        let client = mock.client();
        let request = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
        let grant = mock
            .runtime
            .block_on(client.exchange("callback-code", &request.verifier, SETUP_SCOPES))
            .unwrap();
        mock.runtime
            .block_on(prepare(
                &repository,
                registry,
                &operation,
                &draft.draft_id,
                &client,
                &grant,
                Some("shared"),
                Some("Team notes"),
                None,
            ))
            .unwrap();
        let prepared = get_storage_setup_draft(&repository, registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.settings.get_str("namespace_id"), Some("shared"));
        assert_eq!(prepared.settings.get_str("folder_name"), Some("Team notes"));
        assert_eq!(mock.count("files/create_folder_v2"), 0);
        assert_eq!(mock.count("sharing/share_folder"), 0);
        assert!(repository.load_config().unwrap().storages.is_empty());
    }
}
