use clap::{Args, Parser, Subcommand};

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
    /// Fill an open R2 setup draft through browser authorization.
    Setup(SetupR2Args),
}

#[derive(Args)]
pub(crate) struct SetupR2Args {
    /// Exact open Mallard storage setup draft to fill.
    #[arg(long)]
    pub draft_id: String,

    /// Draft revision copied from Mallard; concurrent form edits fail closed.
    #[arg(long)]
    pub expected_revision: u64,

    /// Exact storage name shown in the open Mallard form.
    #[arg(long)]
    pub name: String,
}

#[derive(Args)]
pub(crate) struct VerifyArgs {
    /// Mallard storage ID to verify.
    pub storage_id: String,
}
