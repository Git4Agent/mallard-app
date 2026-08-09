mod args;
mod backend;
mod error;
mod output;

use args::{Cli, Command, JurisdictionArg, R2Command, StorageCommand};
use backend::{CleanupInput, ConnectInput, ProductionBackend, ProvisionInput, StorageBackend};
use clap::{error::ErrorKind, Parser};
use mallard_application::storage_setup::{parse_r2_api_url, R2Jurisdiction, SecretString};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use zeroize::Zeroize;

pub use error::{
    EXIT_CLEANUP_REQUIRED, EXIT_CONFLICT, EXIT_INVALID_INPUT, EXIT_PERSISTENCE_FAILURE,
    EXIT_REMOTE_FAILURE, EXIT_SUCCESS,
};

const MAX_SECRET_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecretKind {
    R2SecretAccessKey,
    CloudflareBootstrapToken,
}

impl SecretKind {
    fn prompt(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "R2 Secret Access Key: ",
            Self::CloudflareBootstrapToken => "Cloudflare bootstrap token: ",
        }
    }

    fn hidden_unavailable(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "The hidden Secret Access Key prompt is unavailable.",
            Self::CloudflareBootstrapToken => {
                "The hidden Cloudflare bootstrap token prompt is unavailable."
            }
        }
    }

    fn read_failed(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "The Secret Access Key could not be read.",
            Self::CloudflareBootstrapToken => "The Cloudflare bootstrap token could not be read.",
        }
    }

    fn too_long(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "The Secret Access Key is too long.",
            Self::CloudflareBootstrapToken => "The Cloudflare bootstrap token is too long.",
        }
    }

    fn invalid_utf8(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "The Secret Access Key must be UTF-8.",
            Self::CloudflareBootstrapToken => "The Cloudflare bootstrap token must be UTF-8.",
        }
    }

    fn multiple_lines(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "Standard input must contain exactly one Secret Access Key.",
            Self::CloudflareBootstrapToken => {
                "Standard input must contain exactly one Cloudflare bootstrap token."
            }
        }
    }

    fn required(self) -> &'static str {
        match self {
            Self::R2SecretAccessKey => "The Secret Access Key is required.",
            Self::CloudflareBootstrapToken => "The Cloudflare bootstrap token is required.",
        }
    }
}

trait SecretSource {
    fn read_hidden(&mut self, kind: SecretKind) -> Result<String, error::CliError>;
    fn read_stdin(&mut self, kind: SecretKind) -> Result<String, error::CliError>;
}

struct TerminalSecretSource;

impl SecretSource for TerminalSecretSource {
    fn read_hidden(&mut self, kind: SecretKind) -> Result<String, error::CliError> {
        let secret = rpassword::prompt_password(kind.prompt())
            .map_err(|_| error::CliError::invalid(kind.hidden_unavailable()))?;
        validate_secret(secret, kind)
    }

    fn read_stdin(&mut self, kind: SecretKind) -> Result<String, error::CliError> {
        let mut bytes = Vec::new();
        if io::stdin()
            .lock()
            .take((MAX_SECRET_BYTES + 2) as u64)
            .read_to_end(&mut bytes)
            .is_err()
        {
            bytes.zeroize();
            return Err(error::CliError::invalid(kind.read_failed()));
        }
        if bytes.len() > MAX_SECRET_BYTES + 1 {
            bytes.zeroize();
            return Err(error::CliError::invalid(kind.too_long()));
        }
        let mut secret = match String::from_utf8(bytes) {
            Ok(secret) => secret,
            Err(error) => {
                let mut bytes = error.into_bytes();
                bytes.zeroize();
                return Err(error::CliError::invalid(kind.invalid_utf8()));
            }
        };
        if secret.ends_with('\n') {
            secret.pop();
            if secret.ends_with('\r') {
                secret.pop();
            }
        }
        if secret.contains(['\r', '\n']) {
            secret.zeroize();
            return Err(error::CliError::invalid(kind.multiple_lines()));
        }
        validate_secret(secret, kind)
    }
}

fn validate_secret(mut secret: String, kind: SecretKind) -> Result<String, error::CliError> {
    if secret.is_empty() {
        secret.zeroize();
        return Err(error::CliError::invalid(kind.required()));
    }
    if secret.len() > MAX_SECRET_BYTES {
        secret.zeroize();
        return Err(error::CliError::invalid(kind.too_long()));
    }
    Ok(secret)
}

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
    execute(
        cli,
        &backend,
        &mut TerminalSecretSource,
        &mut stdout,
        &mut stderr,
    )
}

#[cfg(test)]
fn run<I, T>(
    args: I,
    backend: &impl StorageBackend,
    secrets: &mut impl SecretSource,
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
    execute(cli, backend, secrets, stdout, stderr)
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
    secrets: &mut impl SecretSource,
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
                R2Command::Connect(args) => {
                    let name = args.name.clone();
                    parse_r2_api_url(&args.api_url)
                        .map_err(error::CliError::from_setup)
                        .and_then(|_| {
                            read_secret(secrets, args.secret_stdin, SecretKind::R2SecretAccessKey)
                        })
                        .and_then(|secret| {
                            backend.connect(ConnectInput {
                                name,
                                api_url: args.api_url,
                                access_key_id: args.access_key_id,
                                secret_access_key: SecretString::new(secret),
                            })
                        })
                        .and_then(|outcome| {
                            write_result(output::connect(stdout, cli.json, &args.name, &outcome))
                        })
                }
                R2Command::Provision(args) => read_secret(
                    secrets,
                    args.bootstrap_token_stdin,
                    SecretKind::CloudflareBootstrapToken,
                )
                .and_then(|bootstrap_token| {
                    backend.provision(ProvisionInput {
                        name: args.name,
                        account_id: args.account_id,
                        bucket: args.bucket,
                        jurisdiction: match args.jurisdiction {
                            JurisdictionArg::Default => R2Jurisdiction::Default,
                            JurisdictionArg::Eu => R2Jurisdiction::Eu,
                            JurisdictionArg::Fedramp => R2Jurisdiction::Fedramp,
                        },
                        bootstrap_token: SecretString::new(bootstrap_token),
                    })
                })
                .and_then(|outcome| write_result(output::provision(stdout, cli.json, &outcome))),
                R2Command::Cleanup(args) => read_secret(
                    secrets,
                    args.bootstrap_token_stdin,
                    SecretKind::CloudflareBootstrapToken,
                )
                .and_then(|bootstrap_token| {
                    backend.cleanup(CleanupInput {
                        setup_id: args.setup_id,
                        bootstrap_token: SecretString::new(bootstrap_token),
                    })
                })
                .and_then(|outcome| write_result(output::cleanup(stdout, cli.json, &outcome))),
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

fn read_secret(
    secrets: &mut impl SecretSource,
    from_stdin: bool,
    kind: SecretKind,
) -> Result<String, error::CliError> {
    if from_stdin {
        secrets.read_stdin(kind)
    } else {
        secrets.read_hidden(kind)
    }
}

fn write_result(result: io::Result<()>) -> Result<(), error::CliError> {
    result.map_err(|_| error::CliError::persistence("Mallard CLI output could not be written."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        CleanupOutcome, ConnectOutcome, ProvisionOutcome, StorageSummary, VerifyOutcome,
    };
    use crate::error::CliError;

    struct FakeBackend;

    impl StorageBackend for FakeBackend {
        fn list(&self) -> Result<Vec<StorageSummary>, CliError> {
            Ok(vec![StorageSummary {
                storage_id: "storage-safe".to_string(),
                name: "Team backup".to_string(),
                kind: "r2",
            }])
        }

        fn connect(&self, _input: ConnectInput) -> Result<ConnectOutcome, CliError> {
            Ok(ConnectOutcome {
                storage_id: "storage-safe".to_string(),
                config_revision: 12,
                verified: true,
            })
        }

        fn provision(&self, _input: ProvisionInput) -> Result<ProvisionOutcome, CliError> {
            Ok(ProvisionOutcome {
                storage_id: "storage-safe".to_string(),
                config_revision: 12,
                verified: true,
                bucket_created: true,
            })
        }

        fn cleanup(&self, input: CleanupInput) -> Result<CleanupOutcome, CliError> {
            Ok(CleanupOutcome {
                setup_id: input.setup_id,
                cleaned: true,
            })
        }

        fn verify(&self, storage_id: &str) -> Result<VerifyOutcome, CliError> {
            Ok(VerifyOutcome {
                storage_id: storage_id.to_string(),
                config_revision: 12,
                verified: true,
            })
        }
    }

    struct FakeSecrets {
        value: String,
        hidden_kinds: Vec<SecretKind>,
        stdin_kinds: Vec<SecretKind>,
    }

    impl SecretSource for FakeSecrets {
        fn read_hidden(&mut self, kind: SecretKind) -> Result<String, CliError> {
            self.hidden_kinds.push(kind);
            Ok(self.value.clone())
        }

        fn read_stdin(&mut self, kind: SecretKind) -> Result<String, CliError> {
            self.stdin_kinds.push(kind);
            Ok(self.value.clone())
        }
    }

    #[test]
    fn connect_json_is_sanitized_and_stdin_is_explicit() {
        let canary = "never-print-this-secret";
        let access_key_canary = "never-print-this-access-key";
        let destination_canary = "00000000000000000000000000000000";
        let mut secrets = FakeSecrets {
            value: canary.to_string(),
            hidden_kinds: Vec::new(),
            stdin_kinds: Vec::new(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            [
                "mallard-cli",
                "storage",
                "r2",
                "connect",
                "--name",
                "Team backup",
                "--api-url",
                "https://00000000000000000000000000000000.r2.cloudflarestorage.com/team",
                "--access-key-id",
                access_key_canary,
                "--secret-stdin",
                "--json",
            ],
            &FakeBackend,
            &mut secrets,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert_eq!(secrets.stdin_kinds, [SecretKind::R2SecretAccessKey]);
        assert!(secrets.hidden_kinds.is_empty());
        let combined = format!(
            "{}{}",
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap()
        );
        assert!(!combined.contains(canary));
        assert!(!combined.contains(access_key_canary));
        assert!(!combined.contains(destination_canary));
        assert!(!combined.contains("cloudflarestorage.com"));
        assert!(combined.contains("\"status\":\"connected\""));
    }

    #[test]
    fn provision_json_is_sanitized_and_uses_the_bootstrap_stdin_source() {
        let token_canary = "bootstrap-token-canary-never-print";
        let account_canary = "0123456789abcdef0123456789abcdef";
        let bucket_canary = "private-bucket-canary";
        let name_canary = "Private destination canary";
        let mut secrets = FakeSecrets {
            value: token_canary.to_string(),
            hidden_kinds: Vec::new(),
            stdin_kinds: Vec::new(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            [
                "mallard-cli",
                "storage",
                "r2",
                "provision",
                "--name",
                name_canary,
                "--account-id",
                account_canary,
                "--bucket",
                bucket_canary,
                "--jurisdiction",
                "eu",
                "--bootstrap-token-stdin",
                "--json",
            ],
            &FakeBackend,
            &mut secrets,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert_eq!(secrets.stdin_kinds, [SecretKind::CloudflareBootstrapToken]);
        assert!(secrets.hidden_kinds.is_empty());
        assert!(stderr.is_empty());
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "{\"schema\":1,\"status\":\"provisioned\",\"storage_id\":\"storage-safe\",\"config_revision\":12,\"verified\":true,\"bucket_created\":true}\n"
        );
        for canary in [token_canary, account_canary, bucket_canary, name_canary] {
            assert!(!String::from_utf8_lossy(&stderr).contains(canary));
        }
    }

    #[test]
    fn cleanup_json_is_sanitized_and_uses_the_hidden_bootstrap_source_by_default() {
        let token_canary = "cleanup-bootstrap-token-canary-never-print";
        let setup_id = "r2setup-0123456789abcdef0123456789abcdef";
        let mut secrets = FakeSecrets {
            value: token_canary.to_string(),
            hidden_kinds: Vec::new(),
            stdin_kinds: Vec::new(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            [
                "mallard-cli",
                "storage",
                "r2",
                "cleanup",
                setup_id,
                "--json",
            ],
            &FakeBackend,
            &mut secrets,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert_eq!(secrets.hidden_kinds, [SecretKind::CloudflareBootstrapToken]);
        assert!(secrets.stdin_kinds.is_empty());
        assert!(stderr.is_empty());
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!(
                "{{\"schema\":1,\"status\":\"cleaned\",\"setup_id\":\"{setup_id}\",\"cleaned\":true}}\n"
            )
        );
        assert!(!String::from_utf8_lossy(&stderr).contains(token_canary));
    }

    #[test]
    fn provision_parses_each_supported_jurisdiction() {
        for (value, expected) in [
            ("default", JurisdictionArg::Default),
            ("eu", JurisdictionArg::Eu),
            ("fedramp", JurisdictionArg::Fedramp),
        ] {
            let cli = Cli::try_parse_from([
                "mallard-cli",
                "storage",
                "r2",
                "provision",
                "--name",
                "Team backup",
                "--account-id",
                "0123456789abcdef0123456789abcdef",
                "--bucket",
                "team-backup",
                "--jurisdiction",
                value,
            ])
            .unwrap();
            let Command::Storage(storage) = cli.command;
            let StorageCommand::R2(r2) = storage.command else {
                panic!("expected R2 command");
            };
            let R2Command::Provision(args) = r2.command else {
                panic!("expected provision command");
            };
            assert_eq!(args.jurisdiction, expected);
        }

        assert!(Cli::try_parse_from([
            "mallard-cli",
            "storage",
            "r2",
            "provision",
            "--name",
            "Team backup",
            "--account-id",
            "0123456789abcdef0123456789abcdef",
            "--bucket",
            "team-backup",
            "--jurisdiction",
            "invalid",
        ])
        .is_err());
    }

    struct CleanupRequiredBackend;

    impl StorageBackend for CleanupRequiredBackend {
        fn list(&self) -> Result<Vec<StorageSummary>, CliError> {
            unreachable!()
        }

        fn connect(&self, _input: ConnectInput) -> Result<ConnectOutcome, CliError> {
            unreachable!()
        }

        fn provision(&self, _input: ProvisionInput) -> Result<ProvisionOutcome, CliError> {
            unreachable!()
        }

        fn cleanup(&self, input: CleanupInput) -> Result<CleanupOutcome, CliError> {
            Err(CliError::for_test(
                EXIT_CLEANUP_REQUIRED,
                "r2_probe_cleanup_required",
                "R2 remote cleanup is required.",
                Some(&input.setup_id),
            ))
        }

        fn verify(&self, _storage_id: &str) -> Result<VerifyOutcome, CliError> {
            unreachable!()
        }
    }

    #[test]
    fn cleanup_required_error_has_stable_structured_fields_and_no_secret() {
        let token_canary = "cleanup-required-token-canary-never-print";
        let setup_id = "r2setup-0123456789abcdef0123456789abcdef";
        let mut secrets = FakeSecrets {
            value: token_canary.to_string(),
            hidden_kinds: Vec::new(),
            stdin_kinds: Vec::new(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            [
                "mallard-cli",
                "storage",
                "r2",
                "cleanup",
                setup_id,
                "--bootstrap-token-stdin",
                "--json",
            ],
            &CleanupRequiredBackend,
            &mut secrets,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit, EXIT_CLEANUP_REQUIRED);
        assert!(stdout.is_empty());
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            format!(
                "{{\"schema\":1,\"status\":\"error\",\"exit_code\":6,\"code\":\"r2_probe_cleanup_required\",\"message\":\"R2 remote cleanup is required.\",\"setup_id\":\"{setup_id}\"}}\n"
            )
        );
        assert_eq!(secrets.stdin_kinds, [SecretKind::CloudflareBootstrapToken]);
        assert!(secrets.hidden_kinds.is_empty());
        assert!(!String::from_utf8_lossy(&stdout).contains(token_canary));
    }

    #[test]
    fn secret_prompts_use_fixed_purpose_specific_labels() {
        assert_eq!(
            SecretKind::R2SecretAccessKey.prompt(),
            "R2 Secret Access Key: "
        );
        assert_eq!(
            SecretKind::CloudflareBootstrapToken.prompt(),
            "Cloudflare bootstrap token: "
        );
    }

    #[test]
    fn malformed_commands_do_not_echo_argument_values() {
        let canary = "argument-secret-canary";
        let mut secrets = FakeSecrets {
            value: String::new(),
            hidden_kinds: Vec::new(),
            stdin_kinds: Vec::new(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(
            ["mallard-cli", "storage", canary],
            &FakeBackend,
            &mut secrets,
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(exit, EXIT_INVALID_INPUT);
        assert!(!String::from_utf8(stderr).unwrap().contains(canary));
    }
}
