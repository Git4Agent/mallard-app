mod args;
mod backend;
mod error;
mod output;
mod r2_browser_setup;

use args::{Cli, Command, R2Command, StorageCommand};
use backend::{ProductionBackend, SetupInput, StorageBackend};
use clap::{error::ErrorKind, Parser};
use mallard_infrastructure::storage_providers::r2::{valid_bucket_name, R2Jurisdiction};
use std::ffi::OsString;
use std::io::{self, Write};

pub use error::{
    EXIT_CLEANUP_REQUIRED, EXIT_CONFLICT, EXIT_INVALID_INPUT, EXIT_PERSISTENCE_FAILURE,
    EXIT_REMOTE_FAILURE, EXIT_SUCCESS,
};

pub fn run_from_env() -> i32 {
    let args = std::env::args_os().collect::<Vec<_>>();
    // Login publishes progress from a reader thread; do not hold stdout locked
    // across the backend operation. Final output follows after that reader exits.
    let mut stdout = io::stdout();
    let mut stderr = io::stderr().lock();
    let cli = match parse(args, &mut stdout, &mut stderr) {
        Ok(cli) => cli,
        Err(exit_code) => return exit_code,
    };
    let backend = match ProductionBackend::from_user_home() {
        Ok(backend) => backend,
        Err(error) => {
            output::error(&mut stderr, cli.json, &error);
            return error.exit_code();
        }
    };
    execute(cli, &backend, &mut stdout, &mut stderr)
}

/// Clap already limits the value; this keeps the typed jurisdiction the only
/// one the backend sees.
fn jurisdiction(value: &str) -> Result<R2Jurisdiction, error::CliError> {
    R2Jurisdiction::parse(value).ok_or_else(|| {
        error::CliError::invalid_code(
            "r2_jurisdiction_invalid",
            "The R2 jurisdiction must be default, eu, us, or fedramp.",
        )
    })
}

/// The rule the bucket's own adapter applies, checked before anything runs.
fn bucket_name(value: Option<String>) -> Result<Option<String>, error::CliError> {
    match value {
        Some(name) if !valid_bucket_name(&name) => Err(error::CliError::invalid_code(
            "r2_bucket_name_invalid",
            "The R2 bucket name must use 3 to 63 lowercase letters, numbers, or hyphens and begin and end with a letter or number.",
        )),
        value => Ok(value),
    }
}

#[cfg(test)]
fn run<I, T>(
    args: I,
    backend: &impl StorageBackend,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match parse(args, stdout, stderr) {
        Ok(cli) => cli,
        Err(exit_code) => return exit_code,
    };
    execute(cli, backend, stdout, stderr)
}

fn parse<I, T>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<Cli, i32>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let raw_args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let json_requested = raw_args.iter().any(|arg| arg == "--json");
    match Cli::try_parse_from(raw_args) {
        Ok(cli) => Ok(cli),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = write!(stdout, "{error}");
            Err(EXIT_SUCCESS)
        }
        Err(_) => {
            let error =
                error::CliError::invalid("Invalid command. Run 'mallard-cli --help' for usage.");
            output::error(stderr, json_requested, &error);
            Err(error.exit_code())
        }
    }
}

fn execute(
    cli: Cli,
    backend: &impl StorageBackend,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let result = match cli.command {
        Command::Storage(storage) => match storage.command {
            StorageCommand::List => backend
                .list()
                .and_then(|storages| write_result(output::list(stdout, cli.json, &storages))),
            StorageCommand::Verify(args) => backend
                .verify(&args.storage_id)
                .and_then(|outcome| write_result(output::verify(stdout, cli.json, &outcome))),
            StorageCommand::R2(r2) => match r2.command {
                R2Command::Discover(args) => backend
                    .discover_r2(&args.account_id)
                    .and_then(|scan| write_result(output::discovery(stdout, cli.json, &scan))),
                R2Command::Accounts(args) => backend.r2_accounts(args.login).and_then(|accounts| {
                    write_result(output::accounts(stdout, cli.json, &accounts))
                }),
                R2Command::Setup(args) => {
                    let existing = args.existing_bucket.zip(args.creation_date).map(
                        |(name, creation_date)| backend::ExistingBucket {
                            name,
                            creation_date,
                        },
                    );
                    jurisdiction(&args.jurisdiction)
                        .and_then(|jurisdiction| Ok((jurisdiction, bucket_name(args.bucket)?)))
                        .and_then(|(jurisdiction, bucket)| {
                            backend
                                .setup(
                                    SetupInput {
                                        name: args.draft.name,
                                        draft_id: args.draft.draft_id,
                                        expected_revision: args.draft.expected_revision,
                                    },
                                    args.login,
                                    args.account_id,
                                    jurisdiction,
                                    match existing {
                                        Some(bucket) => backend::R2Target::Existing(bucket),
                                        None => backend::R2Target::New { bucket },
                                    },
                                )
                                .and_then(|outcome| {
                                    write_result(output::setup(stdout, cli.json, &outcome))
                                })
                        })
                }
            },
        },
    };

    match result {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => {
            output::error(stderr, cli.json, &error);
            error.exit_code()
        }
    }
}

fn write_result(result: io::Result<()>) -> Result<(), error::CliError> {
    result.map_err(|_| error::CliError::persistence("Mallard CLI output could not be written."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{SetupOutcome, StorageSummary, VerifyOutcome};
    use crate::error::CliError;

    struct FakeBackend;

    #[test]
    fn explicit_login_is_available_only_for_r2_setup() {
        for login in [false, true] {
            let mut args = vec![
                "mallard-cli",
                "storage",
                "r2",
                "setup",
                "--name",
                "Test",
                "--draft-id",
                "storagedraft-111111111111111111111111",
                "--expected-revision",
                "1",
            ];
            if login {
                args.push("--login");
            }
            let cli = Cli::try_parse_from(&args).unwrap();
            let Command::Storage(storage) = cli.command;
            let StorageCommand::R2(r2) = storage.command else {
                panic!("expected R2")
            };
            let R2Command::Setup(setup) = r2.command else {
                panic!("expected setup")
            };
            assert_eq!(setup.login, login);
            args[2] = "dropbox";
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn setup_accepts_only_the_four_jurisdictions_and_defaults_to_the_account_one() {
        let base = [
            "mallard-cli",
            "storage",
            "r2",
            "setup",
            "--name",
            "Test",
            "--draft-id",
            "storagedraft-111111111111111111111111",
            "--expected-revision",
            "1",
        ];
        let parsed = |extra: &[&str]| {
            Cli::try_parse_from(base.into_iter().chain(extra.iter().copied())).map(|cli| {
                let Command::Storage(storage) = cli.command;
                let StorageCommand::R2(r2) = storage.command else {
                    panic!("expected R2")
                };
                let R2Command::Setup(setup) = r2.command else {
                    panic!("expected setup")
                };
                setup.jurisdiction
            })
        };
        assert_eq!(parsed(&[]).unwrap(), "default");
        for value in ["default", "eu", "us", "fedramp"] {
            assert_eq!(parsed(&["--jurisdiction", value]).unwrap(), value);
            assert!(jurisdiction(value).is_ok());
        }
        assert!(parsed(&["--jurisdiction", "fedramp-high"]).is_err());
        assert_eq!(
            jurisdiction("fedramp-high").unwrap_err().code(),
            "r2_jurisdiction_invalid"
        );
    }

    #[test]
    fn existing_r2_requires_account_bucket_and_creation_date_together() {
        let base = [
            "mallard-cli",
            "storage",
            "r2",
            "setup",
            "--name",
            "Test",
            "--draft-id",
            "storagedraft-111111111111111111111111",
            "--expected-revision",
            "1",
        ];
        for extra in [
            vec!["--existing-bucket", "known"],
            vec!["--creation-date", "2026-09-10T00:00:00Z"],
            vec![
                "--existing-bucket",
                "known",
                "--creation-date",
                "2026-09-10T00:00:00Z",
            ],
        ] {
            assert!(Cli::try_parse_from(base.into_iter().chain(extra)).is_err());
        }
        assert!(Cli::try_parse_from(base.into_iter().chain([
            "--existing-bucket",
            "known",
            "--creation-date",
            "2026-09-10T00:00:00Z",
            "--account-id",
            "11111111111111111111111111111111"
        ]))
        .is_ok());
        // A name for a new bucket and an existing bucket are two different asks.
        assert!(Cli::try_parse_from(base.into_iter().chain([
            "--existing-bucket",
            "known",
            "--creation-date",
            "2026-09-10T00:00:00Z",
            "--account-id",
            "11111111111111111111111111111111",
            "--bucket",
            "team-backups"
        ]))
        .is_err());
    }

    #[test]
    fn a_new_bucket_name_is_checked_before_the_backend_runs() {
        assert_eq!(bucket_name(None).unwrap(), None);
        assert_eq!(
            bucket_name(Some("team-backups".into())).unwrap().as_deref(),
            Some("team-backups")
        );
        for invalid in ["Team-Backups", "ab", "-team", "team-", "team_backups", ""] {
            assert_eq!(
                bucket_name(Some(invalid.into())).unwrap_err().code(),
                "r2_bucket_name_invalid",
                "{invalid}"
            );
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            [
                "mallard-cli",
                "--json",
                "storage",
                "r2",
                "setup",
                "--name",
                "Test",
                "--draft-id",
                "storagedraft-111111111111111111111111",
                "--expected-revision",
                "1",
                "--bucket",
                "Team_Backups",
            ],
            &FakeBackend,
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(exit, EXIT_INVALID_INPUT);
        assert!(stdout.is_empty());
        assert!(String::from_utf8_lossy(&stderr).contains("r2_bucket_name_invalid"));
    }

    impl StorageBackend for FakeBackend {
        fn discover_r2(
            &self,
            _account_id: &str,
        ) -> Result<mallard_infrastructure::r2_setup::discovery::BucketScan, CliError> {
            Ok(mallard_infrastructure::r2_setup::discovery::BucketScan {
                buckets: vec![],
                partial: false,
                cut: false,
                jurisdictions: vec!["default".into()],
            })
        }
        fn r2_accounts(
            &self,
            _login: bool,
        ) -> Result<Vec<crate::r2_browser_setup::WranglerAccount>, CliError> {
            Ok(vec![crate::r2_browser_setup::WranglerAccount {
                id: "1".repeat(32),
                name: "Test account".into(),
            }])
        }

        fn list(&self) -> Result<Vec<StorageSummary>, CliError> {
            Ok(vec![StorageSummary {
                storage_id: "storage-safe".to_string(),
                name: "Team backup".to_string(),
                provider: "r2".to_string(),
                provider_label: "Cloudflare R2".to_string(),
            }])
        }

        fn setup(
            &self,
            _input: SetupInput,
            _login: bool,
            _account_id: Option<String>,
            _jurisdiction: R2Jurisdiction,
            _target: backend::R2Target,
        ) -> Result<SetupOutcome, CliError> {
            Ok(SetupOutcome { reconnected: false })
        }

        fn verify(&self, storage_id: &str) -> Result<VerifyOutcome, CliError> {
            Ok(VerifyOutcome {
                storage_id: storage_id.to_string(),
                config_revision: 12,
                verified: true,
            })
        }
    }

    #[test]
    fn account_check_returns_choices_without_preparing_a_draft() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                [
                    "mallard-cli",
                    "--json",
                    "storage",
                    "r2",
                    "accounts",
                    "--login"
                ],
                &FakeBackend,
                &mut stdout,
                &mut stderr
            ),
            EXIT_SUCCESS
        );
        let output: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(output["status"], "accounts");
        assert_eq!(output["accounts"][0]["name"], "Test account");
        assert!(stderr.is_empty());
        let cli = Cli::try_parse_from([
            "mallard-cli",
            "storage",
            "r2",
            "setup",
            "--draft-id",
            "storagedraft-111111111111111111111111",
            "--expected-revision",
            "1",
            "--name",
            "Test",
            "--account-id",
            "11111111111111111111111111111111",
        ])
        .unwrap();
        let Command::Storage(storage) = cli.command;
        let StorageCommand::R2(r2) = storage.command else {
            panic!("expected R2")
        };
        let R2Command::Setup(setup) = r2.command else {
            panic!("expected setup")
        };
        assert_eq!(
            setup.account_id.as_deref(),
            Some("11111111111111111111111111111111")
        );
    }

    #[test]
    fn setup_json_is_sanitized() {
        let name = "Private destination canary";
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            [
                "mallard-cli",
                "storage",
                "r2",
                "setup",
                "--name",
                name,
                "--draft-id",
                "storagedraft-111111111111111111111111",
                "--expected-revision",
                "7",
                "--json",
            ],
            &FakeBackend,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(stderr.is_empty());
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "{\"schema\":1,\"status\":\"draft_ready\",\"reconnected\":false}\n"
        );
        assert!(!String::from_utf8_lossy(&stderr).contains(name));
    }

    #[test]
    fn removed_commands_are_rejected_without_echoing_arguments() {
        for removed in ["connect", "provision", "cleanup"] {
            let canary = "argument-secret-canary";
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run(
                ["mallard-cli", "storage", "r2", removed, canary],
                &FakeBackend,
                &mut stdout,
                &mut stderr,
            );
            assert_eq!(exit, EXIT_INVALID_INPUT);
            assert!(stdout.is_empty());
            assert!(!String::from_utf8(stderr).unwrap().contains(canary));
        }
    }

    #[test]
    fn malformed_commands_do_not_echo_argument_values() {
        let canary = "argument-secret-canary";
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            ["mallard-cli", "storage", canary],
            &FakeBackend,
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(exit, EXIT_INVALID_INPUT);
        assert!(!String::from_utf8(stderr).unwrap().contains(canary));
    }
}
