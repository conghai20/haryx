//! Routes a parsed [`Command`] to its implementation. Commands not ported yet answer "not implemented".

pub(crate) mod accounts;
mod config;
mod guard;
mod listing;

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
        Command::List { token_status, json } => listing::list(token_status, json, debug),
        Command::Status { json } => listing::status(json, debug),
        Command::Switch {
            strategy,
            model,
            json,
        } => listing::switch(strategy, model.as_deref(), json, debug),
        Command::SwitchTo {
            target,
            json,
            force,
        } => listing::switch_to(&target, json, force, debug),
        Command::Add { slot, alias } => accounts::add(slot, alias.as_deref(), debug),
        Command::AddToken { token, email, slot } => {
            accounts::add_token(token.as_deref(), email.as_deref(), slot, debug)
        }
        Command::Remove { account } => accounts::remove(&account, debug),
        Command::Purge => accounts::purge(debug),
        Command::Export {
            path,
            account,
            full,
        } => accounts::export(&path, account.as_deref(), full, debug),
        Command::Import { source, force } => accounts::import(&source, force, debug),
        Command::ImportUsage { path, hold_seconds } => {
            accounts::import_usage(&path, hold_seconds, debug)
        }
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
        Command::Run(opts) => accounts::run(&opts, debug),
        Command::Tui | Command::Watch | Command::Upgrade | Command::Auto(_) => {
            Err(not_implemented(&command))
        }
    }
}

fn not_implemented(command: &Command) -> CommandError {
    CommandError {
        message: format!("{PROG} {}: not implemented yet", command.name()),
        exit_code: NOT_IMPLEMENTED_EXIT,
    }
}
