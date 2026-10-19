use crate::help::examples;
use clap::{Args, Parser, Subcommand};

const ABOUT: &str = "Mallard from the terminal, for people and coding agents.";

/// Bold headings, plain command names and flags, no underline: the look
/// `gh` has.
const STYLES: clap::builder::Styles = clap::builder::Styles::styled()
    .header(clap::builder::styling::Style::new().bold())
    .usage(clap::builder::styling::Style::new().bold())
    .literal(clap::builder::styling::Style::new())
    .placeholder(clap::builder::styling::Style::new());

#[derive(Parser)]
#[command(
    name = "mallard-cli",
    version,
    about = ABOUT,
    styles = STYLES,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// One JSON document on stdout and NDJSON events on stderr.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// This computer's identity, its storages and projects, and the project
    /// the current folder belongs to.
    #[command(after_help = examples(&["status", "--json status", "status --project ~/code/notes"]))]
    Status(StatusArgs),
    /// Registered projects: list, add, inspect, rename, remove, inventory,
    /// project files.
    #[command(arg_required_else_help = true, after_help = examples(&["project list", "project add \"$PWD\" --provider codex=\"$HOME/.codex\" --storage Shared", "project show", "project files --storage Shared", "project rename --name notes-laptop --expected-revision 4", "project remove --expected-revision 4 --confirm"]))]
    Project(ProjectArgs),
    /// A project's storage links: list, add, connect to a stored bundle,
    /// remove.
    #[command(arg_required_else_help = true, after_help = examples(&["link list", "link add --storage Shared --pinned", "link connect --storage Shared --bundle-id 6114d0bf429419d68cce84519d6a557d", "link remove --storage Shared --confirm"]))]
    Link(LinkArgs),
    /// Push a project to a storage: review first, then apply what you
    /// approve.
    #[command(arg_required_else_help = true, after_help = examples(&["push review", "push apply", "push apply --files", "push apply --session codex:conversation:thread-2f1c", "push status", "push finish"]))]
    Push(PushArgs),
    /// Pull a project from a storage: review first, then apply the actions
    /// you approve.
    #[command(arg_required_else_help = true, after_help = examples(&["--json pull review > review.json", "pull apply --review review.json --approve all", "pull apply --review review.json --resource project:content-file:4e2a", "pull readiness"]))]
    Pull(PullArgs),
    /// A project's agent sessions: list, inspect, read a timeline, run a
    /// session action, delete.
    #[command(arg_required_else_help = true, after_help = examples(&["session list", "session list --storage Shared --refresh", "session show codex:conversation:thread-2f1c", "session timeline codex:conversation:thread-2f1c --kind prompt --kind response", "session event codex:conversation:thread-2f1c event-0042", "session delete codex:conversation:thread-2f1c --confirm"]))]
    Session(SessionArgs),
    /// Storages: list, inspect, verify, rename, remove; providers; the R2
    /// setup helper the desktop drives.
    #[command(arg_required_else_help = true, after_help = examples(&["storage list", "storage add local --path /Volumes/shared/mallard --name Shared", "storage add cloud", "storage snapshots Shared", "storage verify Shared", "storage remove Shared --expected-revision 3 --confirm"]))]
    Storage(StorageArgs),
    /// The Library: user-global skills, plugin intent, and Codex pets as
    /// Catalog rows per agent home, synced through a storage with reviews.
    #[command(arg_required_else_help = true, after_help = examples(&["catalog profile add --provider codex --path \"$HOME/.codex\"", "catalog scope add --provider codex --profile <profile-id> --kind agent_skill --format agentskills.io.skill", "catalog link <scope-id> --storage Shared", "catalog items <scope-id>", "catalog push <scope-id> --storage Shared --approve", "catalog pull <scope-id> --storage Shared --approve"]))]
    Catalog(CatalogArgs),
    /// Mallard Cloud: the account's project repositories, website
    /// addresses, versions, and getting a project's folder from one.
    #[command(arg_required_else_help = true, after_help = examples(&["cloud repositories --storage \"Mallard Cloud Rika321\"", "cloud resolve --storage \"Mallard Cloud Rika321\" octocat/ledger", "cloud clone --storage \"Mallard Cloud Rika321\" octocat/ledger --into ~/code/ledger", "cloud publish-version --storage \"Mallard Cloud Rika321\" --name \"Sprint 12\"", "cloud link 'mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Foctocat%2Fledger'"]))]
    Cloud(CloudArgs),
    /// Agent providers (Codex, Claude): descriptors, their command-line
    /// tools, and maintenance actions.
    #[command(arg_required_else_help = true, after_help = examples(&["provider list", "provider cli inspect codex", "provider cli test claude", "provider maintenance codex codex.app.restart --confirm"]))]
    Provider(ProviderArgs),
    /// Mallard's own settings.
    #[command(arg_required_else_help = true, after_help = examples(&["settings get", "settings set credential_redaction_enabled=true", "settings set codex_cli_path=\"/opt/homebrew/bin/codex\" comparison_workers=2"]))]
    Settings(SettingsArgs),
    /// The activity log Mallard keeps of every operation.
    #[command(arg_required_else_help = true, after_help = examples(&["log query --type push --limit 20", "log query --level error", "log stats", "log cleanup --older-than-days 30"]))]
    Log(LogArgs),
}

#[derive(Args)]
pub(crate) struct ProjectSelector {
    /// A project id, or a path inside the project. The current folder when
    /// omitted.
    #[arg(long, global = true, value_name = "ID|PATH")]
    pub project: Option<String>,
}

#[derive(Args)]
pub(crate) struct StatusArgs {
    #[command(flatten)]
    pub selector: ProjectSelector,
}

#[derive(Args)]
pub(crate) struct ProjectArgs {
    #[command(subcommand)]
    pub command: ProjectCommand,
}

#[derive(Subcommand)]
pub(crate) enum ProjectCommand {
    /// Every registered project with its storage links.
    List,
    /// Register a folder as a project on this computer, bound to agent
    /// provider homes and, optionally, linked to a storage.
    #[command(after_help = examples(&["project add \"$PWD\" --provider codex=\"$HOME/.codex\"", "project add ~/code/notes --provider codex --provider claude=\"$HOME/.claude\" --storage Shared --name Notes", "project add ~/code/ledger --storage \"Mallard Cloud Rika321\" --connect 6114d0bf429419d68cce84519d6a557d"]))]
    Add(ProjectAddArgs),
    /// One project: registration, links, binding, materializations.
    Show(ProjectSelector),
    /// The ordinary project files a non-Git project would sync, compared
    /// with a storage.
    Files(ProjectStorageSelector),
    /// Set or clear the machine-local nickname shown instead of the name.
    Rename(ProjectRenameArgs),
    /// Forget the project on this computer. Nothing in any storage changes.
    Remove(ProjectRemoveArgs),
    /// The resources the project's bundle may carry, with the default recipe.
    Inventory(ProjectSelector),
}

#[derive(Args)]
pub(crate) struct ProjectStorageSelector {
    #[command(flatten)]
    pub project: ProjectSelector,
    /// A storage id or name. The project's only link when omitted.
    #[arg(long, global = true, value_name = "ID|NAME")]
    pub storage: Option<String>,
}

#[derive(Args)]
pub(crate) struct ProjectAddArgs {
    /// The project folder.
    #[arg(value_name = "PATH")]
    pub path: String,
    /// An agent provider to bind, as `codex`, `codex=<profile-id>`, or
    /// `codex=<provider-home-path>`. Repeatable. Without it, every provider
    /// the build enables by default is bound to its only saved profile or to
    /// its default home directory.
    #[arg(long, value_name = "KEY[=PROFILE|PATH]")]
    pub provider: Vec<String>,
    /// Link the project to this storage (id or name) at once.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: Option<String>,
    /// Connect to this bundle already in the storage instead of publishing
    /// a new one.
    #[arg(long, value_name = "BUNDLE_ID", requires = "storage")]
    pub connect: Option<String>,
    /// Accept a stored bundle whose Git repository differs from this folder's.
    #[arg(long, requires = "connect")]
    pub allow_repository_mismatch: bool,
    /// The project's name; the folder's name when omitted. A stored bundle
    /// keeps the name it was published with, so with --connect this is the
    /// nickname shown on this computer.
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args)]
pub(crate) struct ProjectRenameArgs {
    #[command(flatten)]
    pub selector: ProjectSelector,
    /// The nickname. Omit it with --clear to show the project's own name again.
    #[arg(long, conflicts_with = "clear")]
    pub name: Option<String>,
    /// Remove the nickname.
    #[arg(long)]
    pub clear: bool,
    /// The project's revision as `project show` printed it.
    #[arg(long)]
    pub expected_revision: u64,
}

#[derive(Args)]
pub(crate) struct ProjectRemoveArgs {
    #[command(flatten)]
    pub selector: ProjectSelector,
    /// The project's revision as `project show` printed it.
    #[arg(long)]
    pub expected_revision: u64,
    /// Required: forget the project on this computer.
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct StorageArgs {
    #[command(subcommand)]
    pub command: StorageCommand,
}

#[derive(Subcommand)]
pub(crate) enum StorageCommand {
    /// Every configured storage, without credentials.
    List,
    /// Set up a new storage and save it: a local folder, or a browser
    /// sign-in to Cloudflare R2, Dropbox, Feishu, or Mallard Cloud.
    #[command(arg_required_else_help = true, after_help = examples(&["storage add local --path /Volumes/shared/mallard --name Shared", "storage add r2 --bucket team-backup", "storage add dropbox --folder-name \"Mallard team\"", "storage add feishu --folder-link https://example.feishu.cn/drive/folder/fldcn...", "storage add cloud"]))]
    Add(StorageAddArgs),
    /// Sign in again for a storage whose access was lost, and save the new
    /// credential. The destination stays the same.
    Reconnect(StorageRef),
    /// Storage setup drafts the desktop or an earlier verb left open.
    #[command(arg_required_else_help = true)]
    Draft(StorageDraftArgs),
    /// One storage's summary, settings, and secret presence.
    Show(StorageRef),
    /// Verify a storage's required remote capabilities.
    Verify(StorageRef),
    /// Rename a storage.
    Rename(StorageRenameArgs),
    /// Remove a storage from this computer. Its objects stay where they are.
    Remove(StorageRemoveArgs),
    /// The storage providers this build offers, with their fields.
    Providers,
    /// The bundles a storage holds.
    Snapshots(StorageRef),
    /// Delete every Mallard-managed object in a storage and reset every link
    /// to it on this computer.
    ClearData(StorageRemoveArgs),
    /// Manage Cloudflare R2 storage.
    R2(R2Args),
}

#[derive(Args)]
pub(crate) struct StorageAddArgs {
    #[command(subcommand)]
    pub command: StorageAddCommand,
}

#[derive(Subcommand)]
pub(crate) enum StorageAddCommand {
    /// A folder on this computer or a mounted drive.
    Local(StorageAddLocalArgs),
    /// Cloudflare R2 through Wrangler's browser login: create a bucket, or
    /// connect one the scan found.
    R2(StorageAddR2Args),
    /// Dropbox through a browser sign-in: a new shared folder, or an
    /// existing one Mallard already uses.
    Dropbox(StorageAddDropboxArgs),
    /// Feishu Drive through a browser sign-in: a new folder, or one by link.
    Feishu(StorageAddFeishuArgs),
    /// Mallard Cloud: sign in with GitHub in the browser.
    Cloud(StorageAddCloudArgs),
}

#[derive(Args)]
pub(crate) struct StorageAddLocalArgs {
    /// The folder. Created when missing.
    #[arg(long)]
    pub path: String,
    /// The storage's name. The folder's name when omitted.
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args)]
pub(crate) struct StorageAddR2Args {
    /// The storage's name. Named after the bucket when omitted.
    #[arg(long)]
    pub name: Option<String>,
    /// Exact account selected before bucket creation; required when Wrangler
    /// knows several.
    #[arg(long)]
    pub account_id: Option<String>,
    #[arg(long, default_value = "default", value_parser = ["default", "eu", "us", "fedramp"])]
    pub jurisdiction: String,
    /// Exact name for the new bucket. Without it, the helper proposes one.
    #[arg(long, conflicts_with_all = ["existing_bucket", "creation_date"])]
    pub bucket: Option<String>,
    /// Connect this existing bucket (from `storage r2 discover`); never
    /// create one.
    #[arg(long, requires_all = ["creation_date", "account_id"])]
    pub existing_bucket: Option<String>,
    /// The creation date the scan listed for the existing bucket.
    #[arg(long, requires = "existing_bucket")]
    pub creation_date: Option<String>,
    /// Sign in again before setting up.
    #[arg(long)]
    pub login: bool,
}

#[derive(Args)]
pub(crate) struct StorageAddDropboxArgs {
    /// The storage's name. Named after the folder when omitted.
    #[arg(long)]
    pub name: Option<String>,
    /// Create a shared folder with this name. Without it and without
    /// --existing, a name is proposed.
    #[arg(long, conflicts_with = "existing")]
    pub folder_name: Option<String>,
    /// Connect a shared folder Mallard already uses, by its name.
    #[arg(long, value_name = "FOLDER_NAME")]
    pub existing: Option<String>,
    /// Let the browser choose another Dropbox account.
    #[arg(long)]
    pub switch_account: bool,
}

#[derive(Args)]
pub(crate) struct StorageAddFeishuArgs {
    /// The storage's name. Named after the folder when omitted.
    #[arg(long)]
    pub name: Option<String>,
    /// Create a folder with this name. Without it and without
    /// --folder-link, a name is proposed.
    #[arg(long, conflicts_with = "folder_link")]
    pub folder_name: Option<String>,
    /// Use an existing Feishu Drive folder by its link.
    #[arg(long)]
    pub folder_link: Option<String>,
}

#[derive(Args)]
pub(crate) struct StorageAddCloudArgs {
    /// The storage's name. Named after the account when omitted.
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args)]
pub(crate) struct StorageDraftArgs {
    #[command(subcommand)]
    pub command: StorageDraftCommand,
}

#[derive(Subcommand)]
pub(crate) enum StorageDraftCommand {
    /// Every open draft.
    List,
    /// One draft's safe settings and secret presence.
    Show(DraftRef),
    /// Probe the draft's destination without saving.
    Test(DraftRef),
    /// Probe and save the draft as a storage.
    Commit(DraftCommitArgs),
    /// Discard the draft. Nothing remote is revoked or deleted.
    Discard(DraftRef),
}

#[derive(Args)]
pub(crate) struct DraftRef {
    #[arg(value_name = "DRAFT_ID")]
    pub draft_id: String,
}

#[derive(Args)]
pub(crate) struct DraftCommitArgs {
    #[command(flatten)]
    pub draft: DraftRef,
    /// The draft's revision as `storage draft show` printed it.
    #[arg(long)]
    pub expected_revision: u64,
}

#[derive(Args)]
pub(crate) struct StorageRef {
    /// A storage id or its name.
    #[arg(value_name = "STORAGE")]
    pub storage: String,
}

#[derive(Args)]
pub(crate) struct StorageRenameArgs {
    #[command(flatten)]
    pub storage: StorageRef,
    #[arg(long)]
    pub name: String,
    /// The configuration revision as `status` or `storage list` printed it.
    #[arg(long)]
    pub expected_revision: u64,
}

#[derive(Args)]
pub(crate) struct StorageRemoveArgs {
    #[command(flatten)]
    pub storage: StorageRef,
    /// The configuration revision as `status` or `storage list` printed it.
    #[arg(long)]
    pub expected_revision: u64,
    /// Required: remove the storage from this computer.
    #[arg(long)]
    pub confirm: bool,
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
pub(crate) struct LinkArgs {
    #[command(flatten)]
    pub project: ProjectSelector,
    #[command(subcommand)]
    pub command: LinkCommand,
}

#[derive(Subcommand)]
pub(crate) enum LinkCommand {
    /// The project's storage links.
    List,
    /// Link the project to a storage, publishing a new bundle at the first
    /// Push.
    Add(LinkAddArgs),
    /// Link the project to a bundle the storage already holds.
    Connect(LinkConnectArgs),
    /// Remove the link. The bundle stays in the storage.
    Remove(LinkRemoveArgs),
}

#[derive(Args)]
pub(crate) struct LinkAddArgs {
    /// A storage id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
    /// Make this the project's default storage.
    #[arg(long)]
    pub pinned: bool,
}

#[derive(Args)]
pub(crate) struct LinkConnectArgs {
    /// A storage id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
    /// The stored bundle, as `storage snapshots` prints it.
    #[arg(long)]
    pub bundle_id: String,
    /// Make this the project's default storage.
    #[arg(long)]
    pub pinned: bool,
    /// Accept a bundle whose Git repository differs from this folder's.
    #[arg(long)]
    pub allow_repository_mismatch: bool,
}

#[derive(Args)]
pub(crate) struct LinkRemoveArgs {
    /// A storage id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
    /// Required: remove the link on this computer.
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct PushArgs {
    #[command(flatten)]
    pub target: ProjectStorageSelector,
    #[command(subcommand)]
    pub command: PushCommand,
}

#[derive(Subcommand)]
pub(crate) enum PushCommand {
    /// Compare the project with the storage and print what a Push would
    /// carry. Nothing is pushed.
    Review(PushReviewArgs),
    /// Compare again and push the recommended selection, adjusted by the
    /// flags. Refused when the storage moved on since the project last
    /// pulled (exit 3).
    #[command(after_help = examples(&["push apply", "push apply --files --acknowledge-warnings all", "push apply --select codex:conversation:thread-2f1c --deselect codex:conversation:thread-9b0e", "push apply --remove-file project:content-file:4e2a"]))]
    Apply(PushApplyArgs),
    /// Finish a push whose record an earlier push claimed and never wrote.
    Finish,
    /// Whether a Push may proceed, and how this bundle relates to its
    /// siblings in the storage.
    Status,
}

#[derive(Args)]
pub(crate) struct PushReviewArgs {
    /// Review one session alone, by its resource id.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub session: Option<String>,
}

#[derive(Args)]
pub(crate) struct PushApplyArgs {
    /// Push one session alone, by its resource id.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub session: Option<String>,
    /// Also push this resource (a session or provider resource id from the
    /// review's inventory). Repeatable.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub select: Vec<String>,
    /// Leave this resource out although the review recommends it. Repeatable.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub deselect: Vec<String>,
    /// Include the project's ordinary files (non-Git projects): the files
    /// already in the recipe and every new, unflagged file the scan found.
    #[arg(long)]
    pub files: bool,
    /// Remove this project file or folder from the storage. Repeatable.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub remove_file: Vec<String>,
    /// Acknowledge a flagged file's warning by its digest, or `all` of them.
    /// Repeatable.
    #[arg(long, value_name = "DIGEST|all")]
    pub acknowledge_warnings: Vec<String>,
}

#[derive(Args)]
pub(crate) struct PullArgs {
    #[command(flatten)]
    pub target: ProjectStorageSelector,
    #[command(subcommand)]
    pub command: PullCommand,
}

#[derive(Subcommand)]
pub(crate) enum PullCommand {
    /// Compare the storage's version with this computer and print the Pull
    /// review: every action, with its id. Nothing is applied.
    Review(PullReviewArgs),
    /// Apply the approved actions of a review printed by `pull review`.
    #[command(after_help = examples(&["--json pull review > review.json", "pull apply --review review.json --approve all", "pull apply --review review.json --approve action-bb05134e2cba1e3fdafef607", "pull apply --review review.json --resource project:content-file:4e2a"]))]
    Apply(PullApplyArgs),
    /// Whether this computer is ready to receive the storage's version.
    Readiness,
}

#[derive(Args)]
pub(crate) struct PullReviewArgs {
    /// Review one stored session alone, by its resource id.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub session: Option<String>,
}

#[derive(Args)]
pub(crate) struct PullApplyArgs {
    /// The review document `pull review --json` printed (its `review`
    /// field, or the whole document), or `-` for stdin.
    #[arg(long, value_name = "FILE|-")]
    pub review: String,
    /// An action id to apply, or `all` for every action that needs no
    /// explicit approval. Repeatable.
    #[arg(
        long,
        value_name = "ACTION_ID|all",
        required_unless_present = "resource"
    )]
    pub approve: Vec<String>,
    /// Apply every action of this resource, explicit approval included.
    /// Repeatable; an approval on its own or beside --approve.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub resource: Vec<String>,
}

#[derive(Args)]
pub(crate) struct SessionArgs {
    #[command(flatten)]
    pub target: ProjectStorageSelector,
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Subcommand)]
pub(crate) enum SessionCommand {
    /// The project's sessions, with Git history and the storage comparison
    /// when a storage is named.
    List(SessionListArgs),
    /// One session: metrics, agents, warnings, actions.
    Show(SessionRef),
    /// One page of a session's timeline.
    #[command(after_help = examples(&["session timeline codex:conversation:thread-2f1c", "session timeline codex:conversation:thread-2f1c --kind prompt --kind response --limit 50", "session timeline codex:conversation:thread-2f1c --tool shell --cursor <cursor>"]))]
    Timeline(SessionTimelineArgs),
    /// One timeline event in full.
    Event(SessionEventArgs),
    /// Where a stored session is in this project, if anywhere.
    Find(SessionRef),
    /// Run one of a session's actions, as `session show` lists them.
    Run(SessionRunArgs),
    /// Delete one local session's files.
    Delete(SessionDeleteArgs),
    /// Delete every local session of the project.
    DeleteAll(SessionDeleteAllArgs),
}

#[derive(Args)]
pub(crate) struct SessionListArgs {
    /// The Git branch whose history maps the sessions.
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub window_days: Option<u64>,
    /// Sessions that ended before this Unix time.
    #[arg(long)]
    pub before: Option<u64>,
    /// Compare with the storage as well: a fresh scan and comparison.
    #[arg(long)]
    pub refresh: bool,
}

#[derive(Args)]
pub(crate) struct SessionRef {
    /// A session's resource id, as `session list` prints it.
    #[arg(value_name = "RESOURCE_ID")]
    pub resource_id: String,
}

#[derive(Args)]
pub(crate) struct SessionTimelineArgs {
    #[command(flatten)]
    pub session: SessionRef,
    /// The snapshot `session show` printed; the current one when omitted.
    #[arg(long)]
    pub snapshot_id: Option<String>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    /// Keep events of these kinds: prompt, response, intermediate_step,
    /// checkpoint, tool_call, agent_call.
    #[arg(long = "kind", value_name = "KIND")]
    pub kinds: Vec<String>,
    /// Keep tool calls of these categories: file_edit, shell, read, other.
    #[arg(long = "tool", value_name = "CATEGORY")]
    pub tools: Vec<String>,
    /// Only the events of one turn.
    #[arg(long)]
    pub turn: Option<String>,
}

#[derive(Args)]
pub(crate) struct SessionEventArgs {
    #[command(flatten)]
    pub session: SessionRef,
    #[arg(value_name = "EVENT_ID")]
    pub event_id: String,
    /// The snapshot `session show` printed; the current one when omitted.
    #[arg(long)]
    pub snapshot_id: Option<String>,
}

#[derive(Args)]
pub(crate) struct SessionRunArgs {
    #[command(flatten)]
    pub session: SessionRef,
    #[arg(value_name = "ACTION_ID")]
    pub action_id: String,
}

#[derive(Args)]
pub(crate) struct SessionDeleteArgs {
    #[command(flatten)]
    pub session: SessionRef,
    /// Required: delete the session's local files.
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct SessionDeleteAllArgs {
    /// The project's revision as `project show` printed it.
    #[arg(long)]
    pub expected_revision: u64,
    /// Required: delete every local session of the project.
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct CloudArgs {
    #[command(subcommand)]
    pub command: CloudCommand,
}

#[derive(Subcommand)]
pub(crate) enum CloudCommand {
    /// The project repositories the account belongs to on a Mallard Cloud
    /// storage, each compared with a project's Git remote.
    Repositories(CloudListArgs),
    /// Repositories the account starred on the website.
    Starred(CloudCursorArgs),
    /// Released projects on the service.
    Public(CloudPublicArgs),
    /// One website address, resolved against a Mallard Cloud storage.
    Resolve(CloudAddressArgs),
    /// Join a private repository with an invitation code.
    AcceptInvite(CloudCodeArgs),
    /// Freeze a project repository's draft as its next version.
    PublishVersion(CloudPublishArgs),
    /// Discard a project repository's draft past its latest version.
    DiscardDraft(CloudDiscardArgs),
    /// Get a repository's project folder: clone its GitHub source, or make
    /// the folder. Register it next with `project add --connect`.
    #[command(after_help = examples(&["cloud clone --storage \"Mallard Cloud Rika321\" octocat/ledger --into ~/code/ledger", "cloud clone --storage \"Mallard Cloud Rika321\" https://mallard-ai.com/octocat/ledger --into ~/code/ledger"]))]
    Clone(CloudCloneArgs),
    /// Read a `mallard://` link or a website page address: what it names,
    /// and what each account on its host sees there.
    Link(CloudLinkArgs),
}

#[derive(Args)]
pub(crate) struct CloudStorageArgs {
    /// A Mallard Cloud storage, by id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
    /// Compare Git remotes with this project's (id or path). The current
    /// folder's project when it is one; none otherwise.
    #[arg(long, value_name = "ID|PATH")]
    pub project: Option<String>,
}

#[derive(Args)]
pub(crate) struct CloudListArgs {
    #[command(flatten)]
    pub target: CloudStorageArgs,
}

#[derive(Args)]
pub(crate) struct CloudCursorArgs {
    /// A Mallard Cloud storage, by id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Args)]
pub(crate) struct CloudPublicArgs {
    #[command(flatten)]
    pub listing: CloudCursorArgs,
    /// `newest` or `stars`.
    #[arg(long, default_value = "newest")]
    pub sort: String,
}

#[derive(Args)]
pub(crate) struct CloudAddressArgs {
    #[command(flatten)]
    pub target: CloudStorageArgs,
    /// `owner/name`, or the repository's website address.
    #[arg(value_name = "ADDRESS")]
    pub address: String,
}

#[derive(Args)]
pub(crate) struct CloudCodeArgs {
    #[command(flatten)]
    pub target: CloudStorageArgs,
    #[arg(value_name = "CODE")]
    pub code: String,
}

#[derive(Args)]
pub(crate) struct CloudPublishArgs {
    #[command(flatten)]
    pub target: ProjectStorageSelector,
    /// The version's name; at most 80 characters.
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args)]
pub(crate) struct CloudDiscardArgs {
    #[command(flatten)]
    pub target: ProjectStorageSelector,
    /// The repository revision the status showed.
    #[arg(long)]
    pub expected_repository_revision: u64,
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct CloudCloneArgs {
    /// A Mallard Cloud storage, by id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
    /// `owner/name`, or the repository's website address.
    #[arg(value_name = "ADDRESS")]
    pub address: String,
    /// The folder to make: new, or empty.
    #[arg(long, value_name = "PATH")]
    pub into: String,
}

#[derive(Args)]
pub(crate) struct CloudLinkArgs {
    /// `mallard://open?address=…`, or the page address itself.
    #[arg(value_name = "LINK")]
    pub link: String,
}

#[derive(Args)]
pub(crate) struct CatalogArgs {
    #[command(subcommand)]
    pub command: CatalogCommand,
}

#[derive(Subcommand)]
pub(crate) enum CatalogCommand {
    /// Every (provider, kind, format) this build can hold in the Library.
    Capabilities,
    /// Agent homes the Library reads: list, add, rename, probe, inspect.
    #[command(arg_required_else_help = true)]
    Profile(CatalogProfileArgs),
    /// Catalog rows: one per agent home and resource kind.
    #[command(arg_required_else_help = true)]
    Scope(CatalogScopeArgs),
    /// The Library repositories a storage holds.
    Repositories(CatalogStorageArg),
    /// Link a row to a storage.
    Link(CatalogLinkArgs),
    /// Unlink a row from a storage. The stored items stay.
    Unlink(CatalogUnlinkArgs),
    /// A row's items, compared with a storage when one is linked or named.
    Items(CatalogItemsArgs),
    /// One item's page, or one of its files.
    #[command(arg_required_else_help = true)]
    Item(CatalogItemArgs),
    /// Push a row's items to a storage: print the review, or apply it with
    /// --approve.
    #[command(after_help = examples(&["catalog push <scope-id> --storage Shared", "catalog push <scope-id> --storage Shared --approve", "catalog push <scope-id> --storage Shared --select <resource-id> --acknowledge-warnings all --approve"]))]
    Push(CatalogPushArgs),
    /// Pull a row's items from a storage: print the review, or apply it with
    /// --approve.
    #[command(after_help = examples(&["catalog pull <scope-id> --storage Shared", "catalog pull <scope-id> --storage Shared --approve", "catalog pull <scope-id> --storage Shared --choose <resource-id>=<action-id> --acknowledge-destructive all --approve"]))]
    Pull(CatalogPullArgs),
    /// Copy a local item under a new name: print the review, or apply it.
    Fork(CatalogForkArgs),
    /// Delete a local item (uninstall a plugin): print the review, or apply it.
    Remove(CatalogRemoveArgs),
    /// Finish a Library push whose record an earlier push claimed and never
    /// wrote.
    FinishPush(CatalogLinkArgs),
    /// Mallard Cloud repositories of Library items.
    #[command(arg_required_else_help = true)]
    Cloud(CatalogCloudArgs),
}

#[derive(Args)]
pub(crate) struct CatalogProfileArgs {
    #[command(subcommand)]
    pub command: CatalogProfileCommand,
}

#[derive(Subcommand)]
pub(crate) enum CatalogProfileCommand {
    List,
    /// Save an agent home for the Library.
    Add(CatalogProfilePathArgs),
    /// What saving a path would do, without saving it.
    Probe(CatalogProfilePathArgs),
    Rename(CatalogProfileRenameArgs),
    /// A saved home's holdings per capability.
    Inspect(CatalogProfileRef),
}

#[derive(Args)]
pub(crate) struct CatalogProfilePathArgs {
    /// A provider key, such as `codex` or `claude`.
    #[arg(long)]
    pub provider: String,
    /// The provider home, such as `$HOME/.codex`.
    #[arg(long)]
    pub path: String,
}

#[derive(Args)]
pub(crate) struct CatalogProfileRef {
    #[arg(value_name = "PROFILE_ID")]
    pub profile_id: String,
}

#[derive(Args)]
pub(crate) struct CatalogProfileRenameArgs {
    #[command(flatten)]
    pub profile: CatalogProfileRef,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub expected_revision: u64,
}

#[derive(Args)]
pub(crate) struct CatalogScopeArgs {
    #[command(subcommand)]
    pub command: CatalogScopeCommand,
}

#[derive(Subcommand)]
pub(crate) enum CatalogScopeCommand {
    List,
    /// Add a row: one agent home, one resource kind in one format.
    Add(CatalogScopeAddArgs),
    /// Remove a row. Local items and stored items stay.
    Remove(CatalogScopeRemoveArgs),
}

#[derive(Args)]
pub(crate) struct CatalogScopeAddArgs {
    #[arg(long)]
    pub provider: String,
    /// A saved profile id, as `catalog profile list` prints it.
    #[arg(long, value_name = "PROFILE_ID")]
    pub profile: String,
    /// The resource kind, as `catalog capabilities` prints it.
    #[arg(long)]
    pub kind: String,
    /// The format, as `catalog capabilities` prints it.
    #[arg(long)]
    pub format: String,
    #[arg(long, default_value_t = 1)]
    pub format_version: u32,
}

#[derive(Args)]
pub(crate) struct CatalogScopeRef {
    /// A row's scope id, as `catalog scope list` prints it.
    #[arg(value_name = "SCOPE_ID")]
    pub scope: String,
}

#[derive(Args)]
pub(crate) struct CatalogScopeRemoveArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[arg(long)]
    pub expected_revision: u64,
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct CatalogStorageArg {
    /// A storage id or name.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: String,
}

#[derive(Args)]
pub(crate) struct CatalogLinkArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[command(flatten)]
    pub storage: CatalogStorageArg,
}

#[derive(Args)]
pub(crate) struct CatalogUnlinkArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct CatalogItemsArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    /// A storage id or name. The row's linked storage when omitted.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: Option<String>,
}

#[derive(Args)]
pub(crate) struct CatalogItemArgs {
    #[command(subcommand)]
    pub command: CatalogItemCommand,
}

#[derive(Subcommand)]
pub(crate) enum CatalogItemCommand {
    /// The item's page: its words, files, and launch lines.
    Show(CatalogItemRef),
    /// One of the item's files, by its exact path.
    File(CatalogItemFileArgs),
}

#[derive(Args)]
pub(crate) struct CatalogItemRef {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[arg(value_name = "RESOURCE_ID")]
    pub resource_id: String,
    /// Read the storage's copy instead of this computer's.
    #[arg(long, value_name = "ID|NAME")]
    pub storage: Option<String>,
}

#[derive(Args)]
pub(crate) struct CatalogItemFileArgs {
    #[command(flatten)]
    pub item: CatalogItemRef,
    #[arg(value_name = "PATH")]
    pub path: String,
}

#[derive(Args)]
pub(crate) struct CatalogPushArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    /// Push this item too, or only these items when any is named. Repeatable.
    #[arg(long, value_name = "RESOURCE_ID")]
    pub select: Vec<String>,
    /// Acknowledge a flagged item's warning by its digest, or `all`.
    #[arg(long, value_name = "DIGEST|all")]
    pub acknowledge_warnings: Vec<String>,
    /// Apply the review in this run instead of printing it.
    #[arg(long)]
    pub approve: bool,
}

#[derive(Args)]
pub(crate) struct CatalogPullArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    /// Pick one action for an item, `RESOURCE_ID=ACTION_ID`; the item's
    /// default action otherwise. Repeatable.
    #[arg(long, value_name = "RESOURCE_ID=ACTION_ID")]
    pub choose: Vec<String>,
    /// Name the local copy a preserve-and-overwrite action keeps,
    /// `RESOURCE_ID=NAME`. Repeatable.
    #[arg(long, value_name = "RESOURCE_ID=NAME")]
    pub copy_name: Vec<String>,
    /// Allow an action that replaces or removes local content, by item id
    /// or `all`. Repeatable.
    #[arg(long, value_name = "RESOURCE_ID|all")]
    pub acknowledge_destructive: Vec<String>,
    /// Apply the review in this run instead of printing it.
    #[arg(long)]
    pub approve: bool,
}

#[derive(Args)]
pub(crate) struct CatalogForkArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[arg(value_name = "RESOURCE_ID")]
    pub resource_id: String,
    /// The copy's name.
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub approve: bool,
}

#[derive(Args)]
pub(crate) struct CatalogRemoveArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[arg(value_name = "RESOURCE_ID")]
    pub resource_id: String,
    #[arg(long)]
    pub approve: bool,
}

#[derive(Args)]
pub(crate) struct CatalogCloudArgs {
    #[command(subcommand)]
    pub command: CatalogCloudCommand,
}

#[derive(Subcommand)]
pub(crate) enum CatalogCloudCommand {
    /// The repository an item is bound to on Mallard Cloud.
    Status(CatalogCloudItemRepositoryArgs),
    /// The account's repositories that could hold this item.
    Repositories(CatalogCloudItemArgs),
    /// Repositories shared with the account.
    Shared(CatalogStorageArg),
    /// Repositories the account starred on the website.
    Starred(CatalogCloudListingArgs),
    /// Released Library items on the service.
    Public(CatalogCloudPublicArgs),
    /// One website address, resolved.
    Resolve(CatalogCloudAddressArgs),
    /// The row that already holds a repository.
    Find(CatalogCloudRepositoryArgs),
    /// Join a private repository with an invitation code.
    Join(CatalogCloudCodeArgs),
    /// Bind an item to an existing repository.
    Connect(CatalogCloudConnectArgs),
    /// Create a repository for an item.
    Create(CatalogCloudCreateArgs),
    /// Discard an item repository's draft past its latest version.
    DiscardDraft(CatalogCloudDiscardArgs),
}

#[derive(Args)]
pub(crate) struct CatalogCloudItemArgs {
    #[command(flatten)]
    pub scope: CatalogScopeRef,
    #[arg(value_name = "RESOURCE_ID")]
    pub resource_id: String,
    #[command(flatten)]
    pub storage: CatalogStorageArg,
}

#[derive(Args)]
pub(crate) struct CatalogCloudItemRepositoryArgs {
    #[command(flatten)]
    pub item: CatalogCloudItemArgs,
    #[arg(long)]
    pub repository_id: String,
}

#[derive(Args)]
pub(crate) struct CatalogCloudListingArgs {
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    /// Limit to one row's coordinate.
    #[arg(long, value_name = "SCOPE_ID")]
    pub scope: Option<String>,
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Args)]
pub(crate) struct CatalogCloudPublicArgs {
    #[command(flatten)]
    pub listing: CatalogCloudListingArgs,
    /// `newest` or `stars`.
    #[arg(long, default_value = "newest")]
    pub sort: String,
}

#[derive(Args)]
pub(crate) struct CatalogCloudAddressArgs {
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    #[arg(value_name = "ADDRESS")]
    pub address: String,
}

#[derive(Args)]
pub(crate) struct CatalogCloudRepositoryArgs {
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    #[arg(value_name = "REPOSITORY_ID")]
    pub repository_id: String,
}

#[derive(Args)]
pub(crate) struct CatalogCloudCodeArgs {
    #[command(flatten)]
    pub storage: CatalogStorageArg,
    #[arg(value_name = "CODE")]
    pub code: String,
}

#[derive(Args)]
pub(crate) struct CatalogCloudConnectArgs {
    #[command(flatten)]
    pub item: CatalogCloudItemArgs,
    #[arg(long)]
    pub repository_id: String,
    /// The row's link revision as `catalog items` prints it.
    #[arg(long)]
    pub expected_link_revision: u64,
}

#[derive(Args)]
pub(crate) struct CatalogCloudCreateArgs {
    #[command(flatten)]
    pub item: CatalogCloudItemArgs,
    /// The row's link revision as `catalog items` prints it.
    #[arg(long)]
    pub expected_link_revision: u64,
}

#[derive(Args)]
pub(crate) struct CatalogCloudDiscardArgs {
    #[command(flatten)]
    pub item: CatalogCloudItemArgs,
    #[arg(long)]
    pub repository_id: String,
    /// The repository revision as `catalog cloud status` printed it.
    #[arg(long)]
    pub expected_revision: u64,
}

#[derive(Args)]
pub(crate) struct ProviderArgs {
    #[command(subcommand)]
    pub command: ProviderCommand,
}

#[derive(Subcommand)]
pub(crate) enum ProviderCommand {
    /// The agent providers this build knows, with their maintenance actions.
    List,
    /// The provider's command-line tool as this computer resolves it.
    #[command(arg_required_else_help = true)]
    Cli(ProviderCliArgs),
    /// Run one of a provider's declared maintenance actions.
    Maintenance(MaintenanceArgs),
}

#[derive(Args)]
pub(crate) struct ProviderCliArgs {
    #[command(subcommand)]
    pub command: ProviderCliCommand,
}

#[derive(Subcommand)]
pub(crate) enum ProviderCliCommand {
    /// Which executable discovery finds, or why none resolved.
    Inspect(ProviderRef),
    /// Run the resolved executable's `--version`.
    Test(ProviderRef),
    /// Forget the probed shell PATH; the next inspection asks the shell again.
    Refresh,
}

#[derive(Args)]
pub(crate) struct ProviderRef {
    /// A provider key, such as `codex` or `claude`.
    #[arg(value_name = "PROVIDER")]
    pub provider: String,
}

#[derive(Args)]
pub(crate) struct MaintenanceArgs {
    #[command(flatten)]
    pub provider: ProviderRef,
    /// The action id, as `provider list` prints it.
    #[arg(value_name = "ACTION")]
    pub action: String,
    /// The action's arguments as one JSON object.
    #[arg(long, value_name = "JSON")]
    pub arguments: Option<String>,
    /// Confirm an action that declares a confirmation prompt.
    #[arg(long)]
    pub confirm: bool,
}

#[derive(Args)]
pub(crate) struct SettingsArgs {
    #[command(subcommand)]
    pub command: SettingsCommand,
}

#[derive(Subcommand)]
pub(crate) enum SettingsCommand {
    /// Every setting as saved.
    Get,
    /// Change settings: `key=value` pairs, with JSON values (`true`, `4`,
    /// `"/usr/local/bin/codex"`) or plain text.
    #[command(after_help = examples(&["settings set credential_redaction_enabled=true", "settings set codex_cli_path=\"/opt/homebrew/bin/codex\"", "settings set codex_cli_path=null comparison_workers=4"]))]
    Set(SettingsSetArgs),
}

#[derive(Args)]
pub(crate) struct SettingsSetArgs {
    #[arg(value_name = "KEY=VALUE", required = true)]
    pub assignments: Vec<String>,
}

#[derive(Args)]
pub(crate) struct LogArgs {
    #[command(subcommand)]
    pub command: LogCommand,
}

#[derive(Subcommand)]
pub(crate) enum LogCommand {
    /// Entries, newest first, one page at a time.
    #[command(after_help = examples(&["log query", "log query --type push --type pull --limit 20", "log query --level error --search \"Push failed\"", "log query --cursor <cursor>"]))]
    Query(LogQueryArgs),
    /// Size, span, and retention policy of the log.
    Stats,
    /// Change the retention policy.
    Policy(LogPolicyArgs),
    /// Delete log files.
    Cleanup(LogCleanupArgs),
    /// Where the log files are.
    Folder,
}

#[derive(Args)]
pub(crate) struct LogQueryArgs {
    /// Keep entries of these types: push, pull, repair, storage,
    /// configuration, history, system.
    #[arg(long = "type", value_name = "TYPE")]
    pub types: Vec<String>,
    /// Keep entries of these levels: info, success, warning, error.
    #[arg(long = "level", value_name = "LEVEL")]
    pub levels: Vec<String>,
    /// Keep entries whose message contains this text.
    #[arg(long)]
    pub search: Option<String>,
    /// Continue from the cursor the previous page printed.
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args)]
pub(crate) struct LogPolicyArgs {
    #[arg(long)]
    pub retention_days: Option<u32>,
    #[arg(long)]
    pub max_total_bytes: Option<u64>,
}

#[derive(Args)]
pub(crate) struct LogCleanupArgs {
    /// Delete every log file.
    #[arg(long, conflicts_with = "older_than_days")]
    pub all: bool,
    /// Delete log files older than this many days.
    #[arg(long)]
    pub older_than_days: Option<u32>,
}
