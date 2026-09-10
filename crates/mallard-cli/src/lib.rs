mod args;
mod backend;
mod dropbox_browser_setup;
mod error;
mod output;
mod r2_browser_setup;

use args::{Cli, Command, DropboxCommand, R2Command, StorageCommand};
use backend::{ProductionBackend, SetupInput, StorageBackend};
use clap::{error::ErrorKind, Parser};
use std::ffi::OsString;
use std::io::{self, Write};

pub use error::{
    EXIT_CLEANUP_REQUIRED, EXIT_CONFLICT, EXIT_INVALID_INPUT, EXIT_PERSISTENCE_FAILURE,
    EXIT_REMOTE_FAILURE, EXIT_SUCCESS,
};

pub fn run_from_env() -> i32 {
    let args = std::env::args_os().collect::<Vec<_>>();
    let mut stdout = io::stdout().lock();
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
                R2Command::Setup(args) => backend
                    .setup(SetupInput {
                        name: args.name,
                        draft_id: args.draft_id,
                        expected_revision: args.expected_revision,
                    })
                    .and_then(|outcome| write_result(output::setup(stdout, cli.json, &outcome))),
            },
            StorageCommand::Dropbox(dropbox) => match dropbox.command {
                DropboxCommand::Setup(args) => backend
                    .setup_dropbox(SetupInput {
                        name: args.name,
                        draft_id: args.draft_id,
                        expected_revision: args.expected_revision,
                    })
                    .and_then(|outcome| write_result(output::setup(stdout, cli.json, &outcome))),
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

    impl StorageBackend for FakeBackend {
        /// The fake treats the draft at revision 2 as an existing storage.
        fn setup_dropbox(&self, input: SetupInput) -> Result<SetupOutcome, error::CliError> {
            Ok(SetupOutcome {
                reconnected: input.expected_revision == 2,
            })
        }
        fn list(&self) -> Result<Vec<StorageSummary>, CliError> {
            Ok(vec![StorageSummary {
                storage_id: "storage-safe".to_string(),
                name: "Team backup".to_string(),
                provider: "r2".to_string(),
                provider_label: "Cloudflare R2".to_string(),
            }])
        }

        fn setup(&self, _input: SetupInput) -> Result<SetupOutcome, CliError> {
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
    fn dropbox_command_requires_exact_draft_and_no_app_key_or_token_argument() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                [
                    "mallard-cli",
                    "storage",
                    "dropbox",
                    "setup",
                    "--name",
                    "Dropbox storage 1",
                    "--draft-id",
                    "storagedraft-111111111111111111111111",
                    "--expected-revision",
                    "1",
                    "--json"
                ],
                &FakeBackend,
                &mut stdout,
                &mut stderr
            ),
            EXIT_SUCCESS
        );
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "{\"schema\":1,\"status\":\"draft_ready\",\"reconnected\":false}\n"
        );
        // Regaining access for an existing storage says so, in both forms,
        // and hands the user Save changes rather than Finish setup.
        for (json, expected) in [
            (
                true,
                "{\"schema\":1,\"status\":\"draft_ready\",\"reconnected\":true}\n".to_string(),
            ),
            (
                false,
                "Access regained for the existing storage. Review the connection in Mallard, optionally test it, then select Save changes.\n".to_string(),
            ),
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut arguments = vec![
                "mallard-cli",
                "storage",
                "dropbox",
                "setup",
                "--name",
                "Dropbox storage 1",
                "--draft-id",
                "storagedraft-111111111111111111111111",
                "--expected-revision",
                "2",
            ];
            if json {
                arguments.push("--json");
            }
            assert_eq!(
                run(arguments, &FakeBackend, &mut stdout, &mut stderr),
                EXIT_SUCCESS
            );
            assert_eq!(String::from_utf8(stdout).unwrap(), expected);
        }
        for argument in [
            "--access-token",
            "--refresh-token",
            "--app-key",
            "--namespace-id",
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run(
                    [
                        "mallard-cli",
                        "storage",
                        "dropbox",
                        "setup",
                        argument,
                        "secret-canary"
                    ],
                    &FakeBackend,
                    &mut stdout,
                    &mut stderr
                ),
                EXIT_INVALID_INPUT
            );
            assert!(!String::from_utf8(stderr).unwrap().contains("secret-canary"));
        }
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
