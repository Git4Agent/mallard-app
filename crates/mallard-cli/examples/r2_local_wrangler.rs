//! Standalone POC. Run `cargo run -p mallard-cli --example r2_local_wrangler -- --help`.
//! Reuse the current auth and callback code without changing production commands.

#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;

#[allow(dead_code)]
mod browser {
    include!("../src/r2_browser_setup.rs");

    // These POC-only adapters use the existing bounded, sanitized process runner.
    impl WranglerClient {
        fn run_status(
            &self,
            args: &[&str],
            account_id: Option<&str>,
            timeout: Duration,
        ) -> Result<bool, ProcessFailure> {
            let mut command = self.command(args, account_id);
            command.stdout(Stdio::null());
            let mut child = command.spawn().map_err(map_spawn_failure)?;
            wait_with_timeout(&mut child, timeout).map(|status| status.success())
        }

        pub(super) fn login_explicitly(&self) -> Result<(), CliError> {
            let success = self
                .run_status(&["login"], None, LOGIN_TIMEOUT)
                .map_err(map_wrangler_auth_process_error)?;
            if !success {
                return Err(CliError::remote(
                    "wrangler_login_failed",
                    "Cloudflare login was not completed.",
                ));
            }
            self.check_existing_login().map(|_| ())
        }

        pub(super) fn check_existing_login(&self) -> Result<String, CliError> {
            match self.whoami()? {
                AuthState::Authenticated(accounts) => select_account(accounts, None),
                AuthState::LoggedOut => Err(CliError::invalid_code(
                    "wrangler_login_required",
                    "Run wrangler login, then run the POC again.",
                )),
            }
        }

        pub(super) fn check_bucket_access(&self, account: &str) -> Result<(), CliError> {
            let success = self
                .run_status(&["r2", "bucket", "list"], Some(account), WHOAMI_TIMEOUT)
                .map_err(map_wrangler_auth_process_error)?;
            if success {
                Ok(())
            } else {
                Err(CliError::remote(
                    "r2_list_failed",
                    "Wrangler could not list R2 buckets.",
                ))
            }
        }

        pub(super) fn prepare_exact_bucket(
            &self,
            account: &str,
            bucket: &str,
            create: bool,
        ) -> Result<(), CliError> {
            if create {
                let success = self
                    .run_status(
                        &["r2", "bucket", "create", bucket, "--no-update-config"],
                        Some(account),
                        WHOAMI_TIMEOUT,
                    )
                    .map_err(|_| uncertain_creation())?;
                if !success {
                    // No retry or replacement name after an unknown remote result.
                    return Err(uncertain_creation());
                }
                super::event("BUCKET_CREATED");
            }
            let success = self
                .run_status(
                    &["r2", "bucket", "info", bucket, "--json"],
                    Some(account),
                    WHOAMI_TIMEOUT,
                )
                .map_err(map_wrangler_auth_process_error)?;
            if !success {
                return Err(CliError::remote(
                    "r2_bucket_check_failed",
                    "Wrangler could not inspect the selected bucket.",
                ));
            }
            Ok(())
        }
    }

    fn uncertain_creation() -> CliError {
        CliError::remote(
            "r2_creation_not_confirmed",
            "Bucket creation was not confirmed. Inspect Cloudflare before another create attempt.",
        )
    }

    #[cfg(all(test, unix))]
    mod poc_process_tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn fixture(create_status: u8) -> (TempDir, WranglerClient) {
            let directory = tempfile::tempdir().unwrap();
            let program = directory.path().join("wrangler");
            std::fs::write(
                &program,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$0.calls\"\n\
                     if [ \"$3\" = create ]; then\n\
                     printf '%s\\n' private-subprocess-canary\n\
                     exit {create_status}\nfi\nexit 0\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            let client = WranglerClient::with_program(program.into_os_string()).unwrap();
            (directory, client)
        }

        #[test]
        fn failed_creation_stops_without_retry_or_bucket_adoption() {
            let (directory, client) = fixture(1);
            let error = client
                .prepare_exact_bucket(&"a".repeat(32), "mallard-poc-fixture", true)
                .unwrap_err();
            assert_eq!(error.code(), "r2_creation_not_confirmed");
            assert!(!format!("{error:?}").contains("private-subprocess-canary"));
            assert_eq!(
                std::fs::read_to_string(directory.path().join("wrangler.calls")).unwrap(),
                "r2\nbucket\ncreate\nmallard-poc-fixture\n--no-update-config\n"
            );
        }

        #[test]
        fn existing_bucket_is_checked_without_a_create_command() {
            let (directory, client) = fixture(0);
            client
                .prepare_exact_bucket(&"a".repeat(32), "existing-fixture", false)
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(directory.path().join("wrangler.calls")).unwrap(),
                "r2\nbucket\ninfo\nexisting-fixture\n--json\n"
            );
        }
    }
}

use clap::{Parser, Subcommand};
use error::CliError;
use mallard_application::storage_setup::{
    begin_storage_draft_operation, create_storage_setup_draft, prepare_storage_draft_in_operation,
    test_storage_setup_draft, PreparedProviderConfig,
};
use mallard_domain::{BoundedConfigObject, SecretMap, SecretValue, StorageSetupDraftId};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::r2::{
    parse_r2_api_url, provider_key, r2_settings_v1, secret_access_key, R2Destination,
};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;
use serde_json::json;

#[derive(Parser)]
#[command(about = "Test local Wrangler R2 setup without an agent or a saved Mallard connection")]
struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Check executable discovery and supported commands without authentication.
    Runtime,
    /// Open Wrangler login in the system browser. Also repairs an expired login.
    Login,
    /// Check the installed Wrangler login, OAuth output, and R2 list access. No browser.
    Check,
    /// Authorize one exact bucket and prepare a draft in a temporary Mallard home.
    Setup {
        /// Existing bucket in the default jurisdiction, or an exact new POC bucket name.
        #[arg(long)]
        bucket: String,
        /// Create the named bucket first. Requires the mallard-poc- prefix. No retry.
        #[arg(long)]
        create: bool,
        /// Run Mallard's real capability probe. This writes and removes probe objects.
        #[arg(long)]
        probe: bool,
    },
}

fn event(status: &'static str) {
    println!("{}", json!({ "status": status, "agent_used": false }));
}

fn local_error() -> CliError {
    CliError::persistence("The POC could not use its temporary Mallard state.")
}

fn destination(account: &str, bucket: &str, create: bool) -> Result<R2Destination, CliError> {
    if create && !bucket.starts_with("mallard-poc-") {
        return Err(CliError::invalid_code(
            "poc_bucket_prefix_required",
            "A new POC bucket must start with mallard-poc-.",
        ));
    }
    parse_r2_api_url(&format!(
        "https://{account}.r2.cloudflarestorage.com/{bucket}"
    ))
    .map_err(|_| CliError::invalid("The selected R2 bucket is invalid."))
}

// This is the ordinary application draft boundary. It never commits a connection.
fn prepare_draft(
    repository: &LocalStateRepository,
    destination: &R2Destination,
    access_key_id: &str,
    secret: SecretValue,
) -> Result<(StorageSetupDraftId, u64), CliError> {
    let registry = StorageProviderRegistry::production();
    let draft = create_storage_setup_draft(repository, registry, &provider_key())
        .map_err(CliError::from_setup)?;
    let operation =
        begin_storage_draft_operation(repository, &draft.draft_id, draft.revision, &draft.name)
            .map_err(CliError::from_setup)?;
    let settings = BoundedConfigObject::encode(&r2_settings_v1(destination, access_key_id))
        .map_err(|_| local_error())?;
    let mut secrets = SecretMap::empty();
    secrets
        .insert(secret_access_key(), secret)
        .map_err(|_| local_error())?;
    let prepared = prepare_storage_draft_in_operation(
        &operation,
        repository,
        registry,
        PreparedProviderConfig { settings, secrets },
    )
    .map_err(CliError::from_setup)?;
    Ok((prepared.draft_id, prepared.draft_revision))
}

fn run(args: Args) -> Result<(), CliError> {
    // Validate local input before any authentication or browser work.
    if let Action::Setup { bucket, create, .. } = &args.command {
        destination(&"a".repeat(32), bucket, *create)?;
    }
    let wrangler = browser::WranglerClient::new()?;
    event("CHECKING_WRANGLER");
    match args.command {
        Action::Runtime => {
            wrangler.check_dependency()?;
            event("RUNTIME_READY");
        }
        Action::Login => {
            event("BROWSER_LOGIN_REQUIRED");
            wrangler.login_explicitly()?;
            event("LOGIN_COMPLETE");
        }
        Action::Check => {
            let account = wrangler.check_existing_login()?;
            event("ACCOUNT_CONFIRMED");
            // The OAuth bearer is NOT an S3 key. Check support; never print it.
            drop(wrangler.oauth_token()?);
            event("OAUTH_AVAILABLE");
            wrangler.check_bucket_access(&account)?;
            event("CHECK_COMPLETE");
        }
        Action::Setup {
            bucket,
            create,
            probe,
        } => {
            // Login, MFA, and consent are completed by the user in the browser.
            let account = wrangler.selected_account_id(None)?;
            let selected = destination(&account, &bucket, create)?;
            wrangler.prepare_exact_bucket(&account, &bucket, create)?;
            event("BUCKET_CONFIRMED");
            event("BROWSER_CONSENT_REQUIRED");
            let credentials = browser::authorize_bucket(selected.account_id(), selected.bucket())?;
            event("S3_CREDENTIALS_RECEIVED");
            let home = tempfile::Builder::new()
                .prefix("mallard-r2-poc-")
                .tempdir()
                .map_err(|_| local_error())?;
            let repository =
                LocalStateRepository::from_home_dir(home.path()).map_err(|_| local_error())?;
            let (draft_id, revision) = prepare_draft(
                &repository,
                &selected,
                &credentials.access_key_id(),
                credentials
                    .secret_access_key()
                    .into_secret_value()
                    .map_err(CliError::from_setup)?,
            )?;
            if probe {
                event("PROBING_S3_CAPABILITIES");
                let runtime = tokio::runtime::Runtime::new().map_err(|_| local_error())?;
                test_storage_setup_draft(
                    &repository,
                    StorageProviderRegistry::production(),
                    runtime.handle(),
                    &draft_id,
                    revision,
                )
                .map_err(CliError::from_setup)?;
                event("S3_PROBE_PASSED");
            }
            let config = repository.load_config().map_err(|_| local_error())?;
            if !config.storages.is_empty() {
                return Err(local_error());
            }
            println!(
                "{}",
                json!({
                    "status": "REVIEW_READY",
                    "agent_used": false,
                    "temporary_draft": true,
                    "draft_revision": revision,
                    "s3_probe_passed": probe,
                    "connection_saved": false,
                    "remote_bucket_created": create,
                    "remote_credential_retained": true
                })
            );
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        // Never print subprocess output, callback URLs, or credential values.
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

    #[test]
    fn prepares_a_real_r2_draft_without_committing_storage() {
        let home = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(home.path()).unwrap();
        let selected = destination(&"a".repeat(32), "mallard-poc-fixture", false).unwrap();
        let (id, revision) = prepare_draft(
            &repository,
            &selected,
            &"b".repeat(32),
            SecretValue::parse(&"c".repeat(64)).unwrap(),
        )
        .unwrap();
        let draft = repository.load_storage_setup_draft(&id).unwrap().unwrap();
        assert_eq!(draft.revision, revision);
        assert!(revision > 1);
        assert!(repository.load_config().unwrap().storages.is_empty());
        let settings: serde_json::Value =
            serde_json::to_value(&draft.storage.provider_config.settings).unwrap();
        assert_eq!(settings["bucket"], "mallard-poc-fixture");
        assert_eq!(settings["api_url"], selected.api_url());
        assert!(draft
            .storage
            .provider_config
            .secrets
            .get(&secret_access_key())
            .is_some());
        let error =
            begin_storage_draft_operation(&repository, &id, revision - 1, &draft.storage.name)
                .err()
                .unwrap();
        assert_eq!(error.code(), "storage_setup_draft_revision_conflict");
    }

    #[test]
    fn rejects_invalid_or_non_poc_create_targets_before_authentication() {
        let account = "a".repeat(32);
        for bucket in [
            "",
            "../other",
            "UpperCase",
            "bucket/other",
            "bucket?query=1",
        ] {
            assert!(destination(&account, bucket, false).is_err());
        }
        assert!(destination(&account, "production-backups", true).is_err());
        assert!(destination(&account, "mallard-poc-valid", true).is_ok());
        assert!(destination(&account, "existing-backups", false).is_ok());
    }
}
