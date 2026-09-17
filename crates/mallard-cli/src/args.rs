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
    Setup(R2SetupArgs),
    /// List available Wrangler accounts before creating a bucket.
    Accounts(R2AccountsArgs),
    /// Find Mallard buckets in the selected account (read-only).
    Discover(R2DiscoverArgs),
}

#[derive(Args)]
pub(crate) struct R2DiscoverArgs {
    #[arg(long)]
    pub account_id: String,
}

#[derive(Args)]
pub(crate) struct R2AccountsArgs {
    /// Open browser login to change the Cloudflare user.
    #[arg(long)]
    pub login: bool,
}

#[derive(Args)]
pub(crate) struct R2SetupArgs {
    #[command(flatten)]
    pub draft: SetupStorageArgs,

    /// Exact account selected before bucket creation.
    #[arg(long)]
    pub account_id: Option<String>,

    /// Where the bucket lives: fixed at creation, and the one an existing
    /// bucket was listed under.
    #[arg(long, default_value = "default", value_parser = ["default", "eu", "us", "fedramp"])]
    pub jurisdiction: String,

    /// Exact name for the new bucket. Without it, the helper proposes one.
    #[arg(long, conflicts_with_all = ["existing_bucket", "creation_date"])]
    pub bucket: Option<String>,

    /// Connect this existing bucket; never create a bucket.
    #[arg(long, requires_all = ["creation_date", "account_id"])]
    pub existing_bucket: Option<String>,

    /// Creation date returned by the scan; rejects a replaced bucket.
    #[arg(long, requires = "existing_bucket")]
    pub creation_date: Option<String>,

    /// Sign in again before preparing this draft.
    #[arg(long)]
    pub login: bool,
}

#[derive(Args)]
pub(crate) struct SetupStorageArgs {
    /// Exact open Mallard storage setup draft to fill.
    #[arg(long)]
    pub draft_id: String,

    /// Draft version copied from Mallard; concurrent form edits fail closed.
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
