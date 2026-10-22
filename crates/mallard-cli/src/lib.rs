mod args;
mod backend;
mod commands;
mod context;
mod error;
mod events;
mod help;
mod output;
mod r2_browser_setup;

use args::{Cli, Command, R2Args, R2Command, StorageCommand};
use backend::{ProductionBackend, SetupInput, StorageBackend};
use clap::{error::ErrorKind, CommandFactory as _, Parser};
use context::CliContext;
use mallard_infrastructure::storage_providers::r2::{valid_bucket_name, R2Jurisdiction};
use std::ffi::OsString;
use std::io::{self, IsTerminal as _, Write};

pub use error::{
    EXIT_CONFLICT, EXIT_INVALID_INPUT, EXIT_PERSISTENCE_FAILURE, EXIT_REMOTE_FAILURE, EXIT_SUCCESS,
};

pub fn run_from_env() -> i32 {
    let args = std::env::args_os().collect::<Vec<_>>();
    // Operations narrate on stderr from their own thread, and the R2 login
    // publishes progress from a reader thread: neither stream may stay
    // locked across a command. Each write takes the lock for itself.
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let styled = io::stdout().is_terminal();
    let cli = match parse(args, styled, &mut stdout, &mut stderr) {
        Ok(cli) => cli,
        Err(exit_code) => return exit_code,
    };
    let json = cli.json;
    let result = match cli.command {
        Command::Storage(args::StorageArgs {
            command: StorageCommand::R2(r2),
        }) => ProductionBackend::from_user_home()
            .and_then(|backend| execute_r2(json, r2, &backend, &mut stdout)),
        command => CliContext::from_user_home(json).and_then(|context| {
            commands::run(command, &context)
                .and_then(|output| write_result(output.write(&mut stdout, json)))
        }),
    };
    finish(result, json, &mut stderr)
}

fn finish(result: Result<(), error::CliError>, json: bool, stderr: &mut dyn Write) -> i32 {
    match result {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => {
            output::error(stderr, json, &error);
            error.exit_code()
        }
    }
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

/// The R2 verbs against an injected backend: the tests' entry.
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
    let cli = match parse(args, false, stdout, stderr) {
        Ok(cli) => cli,
        Err(exit_code) => return exit_code,
    };
    let json = cli.json;
    let result = match cli.command {
        Command::Storage(args::StorageArgs {
            command: StorageCommand::R2(r2),
        }) => execute_r2(json, r2, backend, stdout),
        _ => Err(error::CliError::invalid(
            "Only the R2 verbs run against an injected backend.",
        )),
    };
    finish(result, json, stderr)
}

/// Parse the command line. Help and the version are not errors: the root
/// page (also what a bare `mallard-cli` prints) is laid out in groups, a
/// noun's help is clap's, both bold only when `styled`.
fn parse<I, T>(
    args: I,
    styled: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<Cli, i32>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let raw_args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let json_requested = raw_args.iter().any(|arg| arg == "--json");
    // The root page is asked for when no noun is named: nothing, flags only,
    // or clap's `help` word alone.
    let nouns: Vec<&OsString> = raw_args
        .iter()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .collect();
    let root_page_requested = nouns.is_empty() || (nouns.len() == 1 && nouns[0] == "help");
    match Cli::try_parse_from(raw_args) {
        Ok(cli) => Ok(cli),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) && root_page_requested =>
        {
            let _ = write!(stdout, "{}", help::root_page(&Cli::command(), styled));
            Err(EXIT_SUCCESS)
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                    | ErrorKind::DisplayVersion
            ) =>
        {
            let rendered = error.render();
            let _ = if styled {
                write!(stdout, "{}", rendered.ansi())
            } else {
                write!(stdout, "{rendered}")
            };
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

fn execute_r2(
    json: bool,
    r2: R2Args,
    backend: &impl StorageBackend,
    stdout: &mut dyn Write,
) -> Result<(), error::CliError> {
    match r2.command {
        R2Command::Discover(args) => backend
            .discover_r2(&args.account_id)
            .and_then(|scan| write_result(output::discovery(stdout, json, &scan))),
        R2Command::Accounts(args) => backend
            .r2_accounts(args.login)
            .and_then(|accounts| write_result(output::accounts(stdout, json, &accounts))),
        R2Command::Setup(args) => {
            let existing =
                args.existing_bucket
                    .zip(args.creation_date)
                    .map(|(name, creation_date)| backend::ExistingBucket {
                        name,
                        creation_date,
                    });
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
                        .and_then(|outcome| write_result(output::setup(stdout, json, &outcome)))
                })
        }
    }
}

fn write_result(result: io::Result<()>) -> Result<(), error::CliError> {
    result.map_err(|_| error::CliError::persistence("Mallard CLI output could not be written."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::SetupOutcome;
    use crate::error::CliError;

    struct FakeBackend;

    fn r2_setup(cli: Cli) -> args::R2SetupArgs {
        let Command::Storage(storage) = cli.command else {
            panic!("expected storage")
        };
        let StorageCommand::R2(r2) = storage.command else {
            panic!("expected R2")
        };
        let R2Command::Setup(setup) = r2.command else {
            panic!("expected setup")
        };
        setup
    }

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
            let setup = r2_setup(Cli::try_parse_from(&args).unwrap());
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
            Cli::try_parse_from(base.into_iter().chain(extra.iter().copied()))
                .map(|cli| r2_setup(cli).jurisdiction)
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
        let setup = r2_setup(
            Cli::try_parse_from([
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
            .unwrap(),
        );
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

    #[test]
    fn a_bare_call_prints_the_grouped_page_and_a_bare_noun_its_help() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(["mallard-cli"], &FakeBackend, &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        let page = String::from_utf8(stdout).unwrap();
        assert!(page.contains("CORE COMMANDS\n"), "{page}");
        assert!(page.contains("EXAMPLES\n"), "{page}");
        assert!(!page.contains('\x1b'), "piped output carries no styling");
        assert!(stderr.is_empty());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                ["mallard-cli", "project"],
                &FakeBackend,
                &mut stdout,
                &mut stderr
            ),
            EXIT_SUCCESS
        );
        let page = String::from_utf8(stdout).unwrap();
        assert!(page.contains("Usage: mallard-cli project"), "{page}");
        assert!(page.contains("inventory"), "{page}");
        assert!(stderr.is_empty());
    }

    #[test]
    fn every_noun_has_help() {
        for noun in [
            "status", "project", "storage", "provider", "settings", "log",
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run(
                    ["mallard-cli", noun, "--help"],
                    &FakeBackend,
                    &mut stdout,
                    &mut stderr
                ),
                EXIT_SUCCESS,
                "{noun}"
            );
            assert!(
                String::from_utf8(stdout).unwrap().contains("Usage:"),
                "{noun}"
            );
        }
    }
}
