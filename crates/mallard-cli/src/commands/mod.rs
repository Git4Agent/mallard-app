//! One module per noun. Each verb takes the context and returns the one
//! result the shell prints; narration reaches stderr through the context's
//! sink while the verb runs.

pub(crate) mod catalog;
pub(crate) mod cloud;
pub(crate) mod link;
pub(crate) mod log;
pub(crate) mod project;
pub(crate) mod provider;
pub(crate) mod pull;
pub(crate) mod push;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod status;
pub(crate) mod storage;
pub(crate) mod storage_setup;

use crate::args::Command;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

pub(crate) fn run(command: Command, context: &CliContext) -> Result<CommandOutput, CliError> {
    match command {
        Command::Status(args) => status::status(context, args),
        Command::Project(args) => project::run(context, args.command),
        Command::Link(args) => link::run(context, args),
        Command::Push(args) => push::run(context, args),
        Command::Pull(args) => pull::run(context, args),
        Command::Session(args) => session::run(context, args),
        Command::Storage(args) => storage::run(context, args.command),
        Command::Catalog(args) => catalog::run(context, args.command),
        Command::Cloud(args) => cloud::run(context, args.command),
        Command::Provider(args) => provider::run(context, args.command),
        Command::Settings(args) => settings::run(context, args.command),
        Command::Log(args) => log::run(context, args.command),
    }
}
