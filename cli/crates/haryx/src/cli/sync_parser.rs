//! `haryx sync`: encrypted sync between the machines of one HaryX account.
//! Plan: `docs/haryx-account-plan.md`.

use clap::{Parser, Subcommand};

use super::help::{HELP_TEMPLATE, p};
use super::{Command, Error, Invocation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    Run { json: bool },
    Status { json: bool },
    HasPassphrase { json: bool },
    Unlock { passphrase_stdin: bool, json: bool },
    Passphrase { passphrase_stdin: bool, json: bool },
    Reset { yes: bool, json: bool },
    Forget { email: String, json: bool },
    PrefsGet,
    PrefsSet,
    ProfileList { json: bool },
    ProfilePush { name: String },
    ProfilePull { name: String, force: bool },
    ProfileRemove { name: String },
}

#[derive(Parser, Debug)]
#[command(
    name = "sync",
    bin_name = format!("{} sync", haryx_core::PROG),
    about = p("Sync settings, app preferences, account settings, API-key and setup-token accounts, \
               and setup profiles between your machines, end-to-end encrypted with your sync \
               passphrase. Claude browser logins never leave this machine."),
    after_help = sync_epilog(),
    help_template = HELP_TEMPLATE,
    disable_help_subcommand = true,
    subcommand_value_name = "{status,unlock,passphrase,reset,forget,prefs,profile}",
    subcommand_help_heading = "Actions",
)]
struct SyncArgs {
    /// Print the result as JSON
    #[arg(long, global = true)]
    json: bool,
    /// Enable debug logging
    #[arg(long, global = true)]
    debug: bool,
    #[command(subcommand)]
    action: Option<Verb>,
}

#[derive(Subcommand, Debug)]
enum Verb {
    /// Show whether this machine is unlocked, when it last synced, and what was overwritten
    Status,
    /// Whether this HaryX account has a sync passphrase yet
    #[command(hide = true)]
    HasPassphrase,
    /// Unlock sync on this machine (the first machine sets the passphrase)
    Unlock {
        /// Read the passphrase from the first line of stdin instead of asking
        #[arg(long)]
        passphrase_stdin: bool,
    },
    /// Change the sync passphrase (on an unlocked machine)
    Passphrase {
        /// Read the new passphrase from the first line of stdin instead of asking
        #[arg(long)]
        passphrase_stdin: bool,
    },
    /// Delete everything synced from the server and lock this machine (a forgotten passphrase)
    Reset {
        /// Do not ask for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Stop offering an account (and its token) to your other machines
    Forget {
        #[arg(value_name = "EMAIL")]
        email: String,
    },
    /// The apps' synced preferences (`get` prints them; `set` reads a JSON object on stdin)
    Prefs {
        #[command(subcommand)]
        verb: PrefsVerb,
    },
    /// Setup profiles kept on the server (3 at most)
    Profile {
        #[command(subcommand)]
        verb: ProfileVerb,
    },
}

#[derive(Subcommand, Debug)]
enum PrefsVerb {
    Get,
    Set,
}

#[derive(Subcommand, Debug)]
enum ProfileVerb {
    /// List the profiles on the server
    List,
    /// Upload a saved profile (secrets redacted, no history, no credentials)
    Push { name: String },
    /// Download a profile and import it as a saved profile (it is not applied)
    Pull {
        name: String,
        /// Replace a saved profile of that name
        #[arg(long)]
        force: bool,
    },
    /// Remove a profile from the server
    Remove { name: String },
}

fn sync_epilog() -> String {
    p("Sign in with '{prog} login' first. Each machine unlocks once with the sync passphrase;
HaryX cannot recover it, but any machine still unlocked can set a new one ('{prog} sync passphrase').

Examples:
  {prog} sync unlock
  {prog} sync
  {prog} sync status --json
  {prog} sync profile push work")
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args =
        SyncArgs::try_parse_from(std::iter::once("sync".to_owned()).chain(rest.iter().cloned()))?;
    let json = args.json;
    let action = match args.action {
        None => SyncAction::Run { json },
        Some(Verb::Status) => SyncAction::Status { json },
        Some(Verb::HasPassphrase) => SyncAction::HasPassphrase { json },
        Some(Verb::Unlock { passphrase_stdin }) => SyncAction::Unlock {
            passphrase_stdin,
            json,
        },
        Some(Verb::Passphrase { passphrase_stdin }) => SyncAction::Passphrase {
            passphrase_stdin,
            json,
        },
        Some(Verb::Reset { yes }) => SyncAction::Reset { yes, json },
        Some(Verb::Forget { email }) => SyncAction::Forget { email, json },
        Some(Verb::Prefs { verb }) => match verb {
            PrefsVerb::Get => SyncAction::PrefsGet,
            PrefsVerb::Set => SyncAction::PrefsSet,
        },
        Some(Verb::Profile { verb }) => match verb {
            ProfileVerb::List => SyncAction::ProfileList { json },
            ProfileVerb::Push { name } => SyncAction::ProfilePush { name },
            ProfileVerb::Pull { name, force } => SyncAction::ProfilePull { name, force },
            ProfileVerb::Remove { name } => SyncAction::ProfileRemove { name },
        },
    };
    Ok(Invocation {
        debug: args.debug,
        command: Command::Sync(action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn action(argv: &[&str]) -> Result<SyncAction, Error> {
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        parse(&args).map(|inv| match inv.command {
            Command::Sync(action) => action,
            other => panic!("unexpected {other:?}"),
        })
    }

    #[test]
    fn definition_is_consistent() {
        SyncArgs::command().debug_assert();
    }

    #[test]
    fn bare_sync_runs_and_json_works_anywhere() {
        assert_eq!(action(&[]).unwrap(), SyncAction::Run { json: false });
        assert_eq!(action(&["--json"]).unwrap(), SyncAction::Run { json: true });
        assert_eq!(
            action(&["status", "--json"]).unwrap(),
            SyncAction::Status { json: true }
        );
    }

    #[test]
    fn verbs_parse() {
        assert_eq!(
            action(&["unlock", "--passphrase-stdin", "--json"]).unwrap(),
            SyncAction::Unlock {
                passphrase_stdin: true,
                json: true
            }
        );
        assert_eq!(
            action(&["forget", "a@b.c"]).unwrap(),
            SyncAction::Forget {
                email: "a@b.c".into(),
                json: false
            }
        );
        assert_eq!(action(&["prefs", "set"]).unwrap(), SyncAction::PrefsSet);
        assert_eq!(
            action(&["profile", "pull", "work", "--force"]).unwrap(),
            SyncAction::ProfilePull {
                name: "work".into(),
                force: true
            }
        );
        assert_eq!(
            action(&["has-passphrase", "--json"]).unwrap(),
            SyncAction::HasPassphrase { json: true }
        );
        assert!(action(&["forget"]).is_err());
        assert!(action(&["prefs"]).is_err());
    }
}
