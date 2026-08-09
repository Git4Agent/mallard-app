use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "mallard-cli",
    version,
    about = "Safely manage Mallard from a terminal"
)]
pub(crate) struct Cli {
    #[arg(
        long,
        global = true,
        help = "Emit schema-1 JSON instead of human-readable output"
    )]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Manage storage without displaying credentials or destinations.
    Storage(StorageArgs),
}

#[derive(Args)]
pub(crate) struct StorageArgs {
    #[command(subcommand)]
    pub command: StorageCommand,
}

#[derive(Subcommand)]
pub(crate) enum StorageCommand {
    /// List configured storage.
    List,
    /// Manage Cloudflare R2 storage.
    R2(R2Args),
    /// Verify an existing storage's required remote capabilities.
    Verify(VerifyArgs),
}

#[derive(Args)]
pub(crate) struct R2Args {
    #[command(subcommand)]
    pub command: R2Command,
}

#[derive(Subcommand)]
pub(crate) enum R2Command {
    /// Connect an existing Cloudflare R2 bucket.
    Connect(ConnectR2Args),
    /// Provision and connect a Cloudflare R2 bucket.
    Provision(ProvisionR2Args),
    /// Clean up a recoverable Cloudflare R2 setup.
    Cleanup(CleanupR2Args),
}

#[derive(Args)]
pub(crate) struct ConnectR2Args {
    #[arg(long)]
    pub name: String,

    #[arg(long, value_name = "HTTPS_URL")]
    pub api_url: String,

    #[arg(long, value_name = "ID")]
    pub access_key_id: String,

    #[arg(
        long,
        help = "Read the Secret Access Key from standard input instead of a hidden terminal prompt"
    )]
    pub secret_stdin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum JurisdictionArg {
    Default,
    Eu,
    Fedramp,
}

#[derive(Args)]
pub(crate) struct ProvisionR2Args {
    #[arg(long)]
    pub name: String,

    #[arg(long, value_name = "32_HEX_ACCOUNT_ID")]
    pub account_id: String,

    #[arg(long)]
    pub bucket: String,

    #[arg(long, value_enum)]
    pub jurisdiction: JurisdictionArg,

    #[arg(
        long,
        help = "Read the Cloudflare bootstrap token from standard input instead of a hidden terminal prompt"
    )]
    pub bootstrap_token_stdin: bool,
}

#[derive(Args)]
pub(crate) struct CleanupR2Args {
    /// Recoverable Mallard R2 setup ID.
    pub setup_id: String,

    #[arg(
        long,
        help = "Read the Cloudflare bootstrap token from standard input instead of a hidden terminal prompt"
    )]
    pub bootstrap_token_stdin: bool,
}

#[derive(Args)]
pub(crate) struct VerifyArgs {
    /// Mallard storage ID to verify.
    pub storage_id: String,
}
