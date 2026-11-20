//! Routes a parsed [`Command`] to its implementation. Commands not ported yet answer "not implemented".

mod accounts;
mod config;
mod guard;

use std::fmt;

use haryx_core::PROG;

use crate::cli::Command;

/// Exit code of a command that is not implemented yet (argparse's usage-error code).
pub const NOT_IMPLEMENTED_EXIT: u8 = 2;

/// A command that could not complete: the message for stderr and the process exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError {
    pub message: String,
    pub exit_code: u8,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

impl CommandError {
    /// A failure whose output the command already printed (e.g. a `--json` error envelope on stdout):
    /// only the exit code remains.
    pub fn silent(exit_code: u8) -> Self {
        CommandError {
            message: String::new(),
            exit_code,
        }
    }
}

/// Run one command.
pub fn dispatch(command: Command, debug: bool) -> Result<(), CommandError> {
    match command {
        Command::Config { json, action } => config::run(json, &action),
        Command::Alias(setting) => accounts::alias(&setting, debug),
        Command::Threshold(setting) => accounts::threshold(&setting, debug),
        Command::Priority(setting) => accounts::priority(&setting, debug),
        Command::Swap { first, second } => accounts::swap(&first, &second, debug),
        Command::Move { account, slot } => accounts::move_account(&account, &slot, debug),
        Command::Map { account, path } => accounts::map(account.as_deref(), path.as_deref(), debug),
        Command::Unmap { path } => accounts::unmap(path.as_deref(), debug),
        Command::Unclaimed { purge } => accounts::unclaimed(purge.as_deref(), debug),
        Command::Disable { account } => accounts::set_disabled(&account, true, debug),
        Command::Enable { account } => accounts::set_disabled(&account, false, debug),
        Command::Tui
        | Command::Watch
        | Command::List { .. }
        | Command::Status { .. }
        | Command::Switch { .. }
        | Command::SwitchTo { .. }
        | Command::Add { .. }
        | Command::AddToken { .. }
        | Command::Remove { .. }
        | Command::Export { .. }
        | Command::Import { .. }
        | Command::ImportUsage { .. }
        | Command::Purge
        | Command::Upgrade
        | Command::Run(_)
        | Command::Auto(_) => Err(not_implemented(&command)),
    }
}

fn not_implemented(command: &Command) -> CommandError {
    CommandError {
        message: format!("{PROG} {}: not implemented yet", command.name()),
        exit_code: NOT_IMPLEMENTED_EXIT,
    }
}
