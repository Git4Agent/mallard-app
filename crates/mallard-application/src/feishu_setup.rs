//! Shared Feishu draft preparation for the desktop setup flow: the
//! connection document is written first, then the exact draft.
use crate::storage_setup::{
    prepare_storage_draft_in_operation, PrepareStorageDraftResult, PreparedProviderConfig,
    StorageDraftOperation, StorageSetupError,
};
use mallard_domain::{
    BoundedConfigObject, SecretMap, StorageSetupDraftId, StorageSetupDraftSource,
};
use mallard_infrastructure::dropbox_setup::valid_folder_name;
use mallard_infrastructure::feishu_auth::AuthorizationGrant;
use mallard_infrastructure::feishu_connections::{ConnectionId, FeishuConnectionStore};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::{
    feishu::{provider_key, FeishuSettingsV1},
    StorageProviderRegistry,
};

/// What the sign-in and the folder step produced.
pub struct FeishuPrepared<'a> {
    pub grant: &'a AuthorizationGrant,
    pub account_name: &'a str,
    /// The chosen folder; on a reconnect it must be absent or the saved one.
    pub folder_token: Option<&'a str>,
    /// The name of a folder created for this draft, for listings.
    pub folder_name: Option<&'a str>,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare(
    repository: &LocalStateRepository,
    registry: &StorageProviderRegistry,
    operation: &StorageDraftOperation,
    draft_id: &StorageSetupDraftId,
    connections: &FeishuConnectionStore,
    prepared: FeishuPrepared<'_>,
) -> Result<PrepareStorageDraftResult, StorageSetupError> {
    let invalid = || {
        StorageSetupError::invalid(
            "feishu_setup_identity_mismatch",
            "The Feishu folder does not match this setup.",
        )
    };
    if operation.draft_id() != draft_id
        || operation.provider() != &provider_key()
        || prepared.account_name.trim().is_empty()
        || prepared
            .folder_name
            .is_some_and(|name| !valid_folder_name(name))
    {
        return Err(invalid());
    }
    let draft = repository
        .load_storage_setup_draft(draft_id)
        .map_err(|_| StorageSetupError::persistence())?
        .ok_or_else(invalid)?;
    // A new draft holds a partial form. Only a reconnect has a complete
    // saved connection whose folder and connection ID must be preserved.
    let saved: Option<FeishuSettingsV1> = match draft.source {
        StorageSetupDraftSource::New => None,
        StorageSetupDraftSource::Edit { .. } => Some(
            draft
                .storage
                .provider_config
                .settings
                .decode()
                .map_err(|_| invalid())?,
        ),
    };
    let (folder_token, connection_id) = match &saved {
        // A reconnect keeps the folder and the connection ID.
        Some(saved) => {
            if prepared
                .folder_token
                .is_some_and(|token| token != saved.folder_token)
                || prepared.folder_name.is_some()
            {
                return Err(invalid());
            }
            (
                saved.folder_token.clone(),
                ConnectionId::parse(&saved.connection_id).map_err(|_| invalid())?,
            )
        }
        None => (
            prepared.folder_token.ok_or_else(invalid)?.to_owned(),
            ConnectionId::generate().map_err(|_| StorageSetupError::persistence())?,
        ),
    };
    let config = repository
        .load_config()
        .map_err(|_| StorageSetupError::persistence())?;
    if config.storages.iter().any(|storage| {
        storage.id != draft.storage.id
            && storage.provider == provider_key()
            && storage.provider_config.settings.get_str("folder_token") == Some(&folder_token)
    }) {
        return Err(StorageSetupError::invalid(
            "feishu_already_connected",
            "This Feishu folder is already connected.",
        ));
    }
    // The document first: a draft that names a connection always finds one.
    connections
        .save(&prepared.grant.connection(connection_id.clone()))
        .map_err(|_| {
            StorageSetupError::fixed(
                crate::storage_setup::StorageSetupErrorKind::PersistenceFailure,
                "feishu_connection_not_saved",
                "Mallard could not save the Feishu sign-in on this computer.",
            )
        })?;
    let settings = BoundedConfigObject::encode(&FeishuSettingsV1 {
        connection_id: connection_id.to_string(),
        folder_token,
        folder_name: prepared
            .folder_name
            .map(str::to_owned)
            .or_else(|| saved.as_ref().and_then(|saved| saved.folder_name.clone())),
        account_name: prepared.account_name.trim().to_owned(),
    })
    .map_err(|_| StorageSetupError::persistence())?;
    // No config secret: the app's credentials are the build's, the refresh
    // chain is the document's.
    prepare_storage_draft_in_operation(
        operation,
        repository,
        registry,
        PreparedProviderConfig {
            settings,
            secrets: SecretMap::empty(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_setup::{
        begin_storage_draft_operation, commit_storage_setup_draft, create_storage_edit_draft,
        create_storage_setup_draft, get_storage_setup_draft, remove_storage,
        update_storage_setup_draft, UpdateStorageSetupDraftRequest,
    };
    use mallard_infrastructure::storage_core::feishu_store::mock::{MockFeishuServer, ROOT};

    fn grant(server: &MockFeishuServer) -> AuthorizationGrant {
        let client = server.client();
        server
            .runtime()
            .block_on(client.exchange(&MockFeishuServer::app(), "callback-code", "verifier"))
            .unwrap()
    }

    #[test]
    fn prepare_writes_the_document_then_the_exact_draft_and_refuses_a_connected_folder() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let server = MockFeishuServer::new();
        let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
        // The mock adapter opens stores over the mock's connection documents,
        // as the desktop's installed store and adapter share one root.
        let connections = server.connections.clone();
        let draft = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        // The desktop saves its form before signing in. A new form contains
        // an empty folder field, not a complete saved Feishu connection.
        let draft = update_storage_setup_draft(
            &repository,
            &registry,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: draft.revision,
                name: draft.name.clone(),
                settings: BoundedConfigObject::from_json(&serde_json::json!({
                    "folder_token": "",
                }))
                .unwrap(),
                secrets: Default::default(),
            },
        )
        .unwrap();
        let other = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        let grant = grant(&server);
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
            &connections,
            FeishuPrepared {
                grant: &grant,
                account_name: "Test User",
                folder_token: Some(ROOT),
                folder_name: Some("Mallard fox"),
            },
        )
        .unwrap();
        drop(operation);
        let prepared = get_storage_setup_draft(&repository, &registry, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.revision, draft.revision + 1);
        assert_eq!(prepared.settings.get_str("folder_token"), Some(ROOT));
        assert_eq!(
            prepared.settings.get_str("folder_name"),
            Some("Mallard fox")
        );
        let connection_id =
            ConnectionId::parse(prepared.settings.get_str("connection_id").unwrap()).unwrap();
        let document = connections.load(&connection_id).unwrap().unwrap();
        assert_eq!(document.refresh_token.expose(), "refresh-2");
        assert!(!document.refresh_uncertain);
        assert!(prepared.secrets.is_empty(), "no config secret");
        // The draft holds neither the app's secret nor a token; the config
        // holds nothing yet.
        let text = std::fs::read_to_string(
            temp.path()
                .join(".mallard/storage_drafts")
                .join(format!("{}.json", draft.draft_id)),
        )
        .unwrap();
        assert!(!text.contains(MockFeishuServer::APP_SECRET) && !text.contains("refresh-2"));
        assert_eq!(
            get_storage_setup_draft(&repository, &registry, &other.draft_id)
                .unwrap()
                .unwrap()
                .revision,
            other.revision
        );
        assert!(repository.load_config().unwrap().storages.is_empty());

        // Committed, the folder is connected; a second draft for the same
        // folder is refused before any document is written.
        let committed = commit_storage_setup_draft(
            &repository,
            &registry,
            server.runtime(),
            &draft.draft_id,
            prepared.revision,
            |_| Ok(()),
        )
        .unwrap();
        let operation = begin_storage_draft_operation(
            &repository,
            &other.draft_id,
            other.revision,
            &other.name,
        )
        .unwrap();
        let error = prepare(
            &repository,
            &registry,
            &operation,
            &other.draft_id,
            &connections,
            FeishuPrepared {
                grant: &grant,
                account_name: "Test User",
                folder_token: Some(ROOT),
                folder_name: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "feishu_already_connected");
        drop(operation);
        assert_eq!(
            std::fs::read_dir(connections.root().join("feishu_connections"))
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|e| e == "json"))
                .count(),
            2,
            "the mock's own document and this draft's"
        );

        // A reconnect keeps the connection ID and the folder, and the
        // document is overwritten with the new chain.
        let storage_id = committed.storage_id.clone();
        let edit = create_storage_edit_draft(&repository, &registry, &storage_id).unwrap();
        let operation =
            begin_storage_draft_operation(&repository, &edit.draft_id, edit.revision, &edit.name)
                .unwrap();
        let again = self::grant(&server);
        assert!(
            prepare(
                &repository,
                &registry,
                &operation,
                &edit.draft_id,
                &connections,
                FeishuPrepared {
                    grant: &again,
                    account_name: "Test User",
                    folder_token: Some("fldcnother"),
                    folder_name: None,
                },
            )
            .is_err(),
            "another folder is not a reconnect"
        );
        prepare(
            &repository,
            &registry,
            &operation,
            &edit.draft_id,
            &connections,
            FeishuPrepared {
                grant: &again,
                account_name: "Test User",
                folder_token: None,
                folder_name: None,
            },
        )
        .unwrap();
        drop(operation);
        let reconnected = get_storage_setup_draft(&repository, &registry, &edit.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            reconnected.settings.get_str("connection_id"),
            Some(connection_id.as_str())
        );
        assert_eq!(reconnected.settings.get_str("folder_token"), Some(ROOT));
        assert_eq!(
            reconnected.settings.get_str("folder_name"),
            Some("Mallard fox")
        );
        let document = connections.load(&connection_id).unwrap().unwrap();
        assert_eq!(document.refresh_token.expose(), again.refresh.expose());

        // Removal deletes the document and nothing remote.
        let revision = repository.load_config().unwrap().revision;
        let files = server.file_count();
        // The application removes through the installed store; this process
        // installs none, so the document is removed here as the desktop does.
        remove_storage(&repository, &storage_id, revision, |_| Ok(())).unwrap();
        assert!(connections.remove(&connection_id).unwrap());
        assert_eq!(server.file_count(), files);
    }
}
