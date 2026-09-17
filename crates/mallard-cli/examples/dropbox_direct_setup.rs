//! Direct Dropbox setup POC. Uses a temporary Mallard home and the existing backend.
//! Run `cargo run -p mallard-cli --example dropbox_direct_setup -- setup --probe`.

#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;

#[allow(dead_code)]
#[path = "../src/r2_browser_setup.rs"]
mod r2_browser_setup;

#[allow(dead_code)]
mod backend {
    include!("../src/backend.rs");

fn dropbox_auth_error(error: mallard_infrastructure::dropbox_auth::AuthError) -> CliError {
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
    use mallard_infrastructure::storage_providers::dropbox::provider_key;

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
    runtime.block_on(mallard_application::dropbox_setup::prepare(
        repository, registry, &operation, &draft_id, client, &grant, None, None, Some("Mallard POC"),
    )).map_err(CliError::from_setup)?;
    Ok(SetupOutcome { reconnected })
}


    // POC-only access to the same function used by the production CLI.
    pub(super) fn prepare_direct(
        repository: &LocalStateRepository,
        runtime: &Runtime,
        input: SetupInput,
        client: &mallard_infrastructure::dropbox_auth::DropboxClient,
        authorize: impl FnOnce(
            &mallard_infrastructure::dropbox_auth::AuthorizationRequest,
        ) -> Result<zeroize::Zeroizing<String>, CliError>,
    ) -> Result<SetupOutcome, CliError> {
        prepare_dropbox(
            repository,
            StorageProviderRegistry::production(),
            runtime,
            input,
            client,
            authorize,
        )
    }
}

#[allow(dead_code)]
mod dropbox_browser_setup {
use crate::error::CliError;
use mallard_infrastructure::{
    dropbox_auth::{AuthorizationRequest, CALLBACK_PORT, DROPBOX_CALLBACK},
    loopback_browser,
};
use std::sync::atomic::AtomicBool;
use zeroize::Zeroizing;

pub(crate) fn authorize(request: &AuthorizationRequest) -> Result<Zeroizing<String>, CliError> {
    loopback_browser::authorize(
        &DROPBOX_CALLBACK,
        &request.url,
        &request.state,
        CALLBACK_PORT,
        |url| open::that_detached(url.as_str()).map_err(|_| ()),
        &AtomicBool::new(false),
    ).map_err(|error| CliError::remote(error.code(), error.message()))
}

    #[cfg(test)]
    use std::{io::{Read, Write}, net::TcpStream, time::Duration};

    #[cfg(test)]
    pub(super) fn mock_consent(
        request: &AuthorizationRequest,
        denied: bool,
    ) -> Result<Zeroizing<String>, CliError> {
        let listener = loopback_browser::LoopbackListener::bind(&DROPBOX_CALLBACK, 0).unwrap();
        let address: std::net::SocketAddr = format!("127.0.0.1:{}", listener.port()).parse().unwrap();
        let result = if denied {
            "error=access_denied"
        } else {
            "code=callback-code"
        };
        let callback = format!(
            "GET /dropbox/callback?{result}&state={} HTTP/1.1\r\nHost: {address}\r\n\r\n",
            request.state.as_str()
        );
        std::thread::scope(|scope| {
            let response = scope.spawn(move || {
                // A delayed browser callback must still complete the same attempt.
                std::thread::sleep(Duration::from_millis(75));
                let mut stream = TcpStream::connect(address).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream.write_all(callback.as_bytes()).unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                assert!(!response.contains("callback-code"));
                assert!(response.contains("Cache-Control: no-store"));
            });
            let result = listener
                .receive(&DROPBOX_CALLBACK, &request.url, &request.state, |_| Ok(()), Duration::from_secs(5), &AtomicBool::new(false))
                .map_err(|error| CliError::remote(error.code(), error.message()));
            response.join().unwrap();
            result
        })
    }
}

use clap::{Parser, Subcommand};
use error::CliError;
use mallard_application::storage_setup::{
    create_storage_setup_draft, get_storage_setup_draft, list_storage_setup_drafts,
    test_storage_setup_draft,
};
use mallard_domain::StorageSetupDraftId;
use mallard_infrastructure::dropbox_auth::{AuthorizationRequest, DropboxClient};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::dropbox::{
    provider_key, refresh_token_key, DropboxSettingsV1,
};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;
use serde_json::json;
use tokio::runtime::Runtime;
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(about = "Test direct Dropbox setup without an agent or a saved Mallard connection")]
struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Open browser consent, create an automatically named folder, and prepare a temporary draft.
    Setup {
        /// Run Mallard's real capability probe. This writes and removes probe objects.
        #[arg(long)]
        probe: bool,
    },
}

fn event(status: &'static str) {
    println!("{}", json!({ "status": status, "agent_used": false }));
}

fn local_error() -> CliError {
    CliError::persistence("The POC could not verify its temporary Mallard state.")
}

fn prepare_draft(
    repository: &LocalStateRepository,
    runtime: &Runtime,
    client: &DropboxClient,
    authorize: impl FnOnce(&AuthorizationRequest) -> Result<Zeroizing<String>, CliError>,
) -> Result<(StorageSetupDraftId, u64), CliError> {
    let registry = StorageProviderRegistry::production();
    let initial = create_storage_setup_draft(repository, registry, &provider_key())
        .map_err(CliError::from_setup)?;
    let outcome = backend::prepare_direct(
        repository,
        runtime,
        backend::SetupInput {
            name: initial.name.clone(),
            draft_id: initial.draft_id.to_string(),
            expected_revision: initial.revision,
        },
        client,
        authorize,
    )?;
    let prepared = repository
        .load_storage_setup_draft(&initial.draft_id)
        .map_err(|_| local_error())?
        .ok_or_else(local_error)?;
    let settings: DropboxSettingsV1 = prepared
        .storage
        .provider_config
        .settings
        .decode()
        .map_err(|_| local_error())?;
    let review = get_storage_setup_draft(repository, registry, &initial.draft_id)
        .map_err(CliError::from_setup)?
        .ok_or_else(local_error)?;
    let (listed, warnings) =
        list_storage_setup_drafts(repository, registry).map_err(CliError::from_setup)?;
    if outcome.reconnected
        || prepared.revision <= initial.revision
        || prepared.storage.name != initial.name
        || settings.namespace_id.is_empty()
        || prepared.storage.provider_config.secrets.len() != 1
        || !review.secrets.iter().any(|secret| {
            secret.secret_id == refresh_token_key()
                && secret.value.as_ref().is_some_and(|value| !value.is_empty())
        })
        || !warnings.is_empty()
        || listed.len() != 1
        || listed
            .iter()
            .any(|draft| draft.secrets.iter().any(|secret| secret.value.is_some()))
        || !repository
            .load_config()
            .map_err(|_| local_error())?
            .storages
            .is_empty()
    {
        return Err(local_error());
    }
    Ok((prepared.draft_id, prepared.revision))
}

fn run(args: Args) -> Result<(), CliError> {
    let Action::Setup { probe } = args.command;
    let home = tempfile::Builder::new()
        .prefix("mallard-dropbox-poc-")
        .tempdir()
        .map_err(|_| local_error())?;
    let result = (|| {
        let repository =
            LocalStateRepository::from_home_dir(home.path()).map_err(|_| local_error())?;
        let runtime = Runtime::new().map_err(|_| local_error())?;
        let client = DropboxClient::new()
            .map_err(|error| CliError::remote(error.code(), error.message()))?;
        let (draft_id, revision) = prepare_draft(&repository, &runtime, &client, |request| {
            event("BROWSER_CONSENT_REQUIRED");
            let code = dropbox_browser_setup::authorize(request)?;
            event("CALLBACK_RECEIVED");
            Ok(code)
        })?;
        event("AUTOMATIC_FOLDER_AND_DRAFT_PREPARED");
        if probe {
            event("PROBING_DROPBOX_CAPABILITIES");
            test_storage_setup_draft(
                &repository,
                StorageProviderRegistry::production(),
                runtime.handle(),
                &draft_id,
                revision,
            )
            .map_err(CliError::from_setup)?;
            event("DROPBOX_PROBE_PASSED");
        }
        Ok(revision)
    })();
    // Delete only this POC's temporary home, including its draft and credentials.
    home.close().map_err(|_| local_error())?;
    let revision = result?;
    println!(
        "{}",
        json!({
            "status": "REVIEW_READY",
            "agent_used": false,
            "manual_code_entry": false,
            "automatic_folder_name": true,
            "namespace_populated": true,
            "refresh_credential_populated": true,
            "token_refresh_verified": true,
            "draft_revision": revision,
            "dropbox_probe_passed": probe.then_some(true),
            "connection_saved": false,
            "temporary_state_removed": true,
            "remote_folder_retained": true
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        // Never print OAuth URLs, codes, tokens, account IDs, or folder paths.
        eprintln!(
            "{}",
            json!({ "status": "POC_FAILED", "code": error.code() })
        );
        std::process::exit(error.exit_code());
    }
}

#[cfg(test)]
mod poc_tests {
    use super::*;
    use mallard_infrastructure::dropbox_auth::{CALLBACK_URI, SETUP_SCOPES};
    use mallard_infrastructure::dropbox_setup::mock::MockDropboxSetup;
    use std::collections::BTreeMap;

    #[test]
    fn delayed_browser_consent_fills_review_with_an_automatic_name_and_refresh_token() {
        let home = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(home.path()).unwrap();
        let mock = MockDropboxSetup::new();
        let (id, revision) = prepare_draft(&repository, &mock.runtime, &mock.client(), |request| {
            let params: BTreeMap<_, _> = request.url.query_pairs().into_owned().collect();
            assert_eq!(params["redirect_uri"], CALLBACK_URI);
            assert_eq!(params["token_access_type"], "offline");
            assert_eq!(params["code_challenge_method"], "S256");
            assert_eq!(params["scope"], SETUP_SCOPES);
            assert!(!params.contains_key("client_secret"));
            dropbox_browser_setup::mock_consent(request, false)
        })
        .unwrap();
        assert_eq!(revision, 2);
        let draft = repository.load_storage_setup_draft(&id).unwrap().unwrap();
        assert_eq!(draft.storage.name, "Dropbox storage 1");
        assert!(mock.state.lock().unwrap().create_paths[0].starts_with("/Mallard "));
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        assert_eq!(mock.count("sharing/share_folder"), 1);
        assert_eq!(mock.count("exchange"), 1);
        assert!(mock.count("refresh") >= 1);
        assert!(repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn denied_browser_consent_creates_no_folder_or_credentials() {
        let home = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(home.path()).unwrap();
        let mock = MockDropboxSetup::new();
        let error = prepare_draft(&repository, &mock.runtime, &mock.client(), |request| {
            dropbox_browser_setup::mock_consent(request, true)
        })
        .unwrap_err();
        assert_eq!(error.code(), "dropbox_consent_denied");
        assert!(mock.state.lock().unwrap().counts.is_empty());
        let (drafts, warnings) = repository.list_storage_setup_drafts().unwrap();
        assert!(warnings.is_empty());
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].revision, 1);
        assert!(drafts[0].storage.provider_config.secrets.is_empty());
        assert!(repository.load_config().unwrap().storages.is_empty());
    }
}
