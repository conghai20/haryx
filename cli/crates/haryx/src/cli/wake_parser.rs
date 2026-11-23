//! `haryx ai wake`: start the next 5-hour window right after a reset (plan: `docs/ai-wake-plan.md`).

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};

use super::help::{self, p, sub_bin};
use super::{Command, Error, Invocation};

/// `wake` verbs.
#[derive(Debug, Clone, PartialEq)]
pub enum WakeAction {
    /// Opt accounts in (`all` = every account that exists now).
    On {
        accounts: Vec<String>,
    },
    /// Opt accounts out (`all` clears the list).
    Off {
        accounts: Vec<String>,
    },
    Status {
        json: bool,
    },
    Config {
        quiet: Option<String>,
        no_quiet: bool,
        weekly_cap: Option<f64>,
        json: bool,
    },
    Run {
        once: bool,
        dry_run: bool,
        json: bool,
    },
    Install,
    Uninstall,
}

#[derive(Parser, Debug)]
#[command(
    name = "wake",
    bin_name = sub_bin("wake"),
    about = p("Start the next 5-hour window right after the last one resets, with one \
               1-output-token request, for the accounts you opt in."),
    after_help = wake_epilog(),
    help_template = help::HELP_TEMPLATE,
    disable_help_subcommand = true,
    subcommand_value_name = "{status,on,off,config,run,install,uninstall}",
    subcommand_help_heading = "Actions",
)]
struct WakeArgs {
    /// Enable debug logging
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    action: Option<Verb>,
}

#[derive(Subcommand, Debug)]
enum Verb {
    /// Show each account's wake state (the default)
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Opt accounts in: NUM|EMAIL|ALIAS..., or 'all'
    On {
        #[arg(value_name = "ACCOUNT", required = true)]
        accounts: Vec<String>,
    },
    /// Opt accounts out: NUM|EMAIL|ALIAS..., or 'all'
    Off {
        #[arg(value_name = "ACCOUNT", required = true)]
        accounts: Vec<String>,
    },
    /// Show or change quiet hours and the weekly cap
    Config {
        /// Never wake inside this local range, e.g. 01:00-06:00
        #[arg(long, value_name = "HH:MM-HH:MM", conflicts_with = "no_quiet")]
        quiet: Option<String>,
        /// Remove the quiet hours
        #[arg(long)]
        no_quiet: bool,
        /// Skip a wake when 7-day usage is at or above this percent (default 90)
        #[arg(long, value_name = "PCT")]
        weekly_cap: Option<f64>,
        #[arg(long)]
        json: bool,
    },
    /// Check the opted-in accounts and wake the due ones
    Run {
        /// One check, then exit (what the agent runs)
        #[arg(long)]
        once: bool,
        /// Decide and print, send nothing
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Install the background agent that checks every minute (macOS)
    Install,
    /// Remove the background agent
    Uninstall,
}

fn wake_epilog() -> String {
    p("A wake is skipped when the window already started, inside quiet hours, at or above the weekly
cap, or when the login needs signing in again. Checks with nothing due read local files only.

Examples:
  {ai} wake on 1 work          opt accounts 1 and 'work' in
  {ai} wake on all             opt in every account that exists now
  {ai} wake config --quiet 01:00-06:00 --weekly-cap 85
  {ai} wake install            check every minute in the background
  {ai} wake run --once --dry-run
  {ai} wake status --json")
}

fn usage_error(message: &str) -> Error {
    WakeArgs::command().error(ErrorKind::ArgumentConflict, message)
}

/// `all` stands alone.
fn check_all(accounts: &[String]) -> Result<(), Error> {
    let has_all = accounts
        .iter()
        .any(|a| a.eq_ignore_ascii_case(haryx_core::wake::ALL));
    if has_all && accounts.len() > 1 {
        return Err(usage_error("'all' cannot be combined with account names"));
    }
    Ok(())
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args = WakeArgs::try_parse_from(
        std::iter::once(haryx_core::PROG.to_owned()).chain(rest.iter().cloned()),
    )?;
    let action = match args.action.unwrap_or(Verb::Status { json: false }) {
        Verb::Status { json } => WakeAction::Status { json },
        Verb::On { accounts } => {
            check_all(&accounts)?;
            WakeAction::On { accounts }
        }
        Verb::Off { accounts } => {
            check_all(&accounts)?;
            WakeAction::Off { accounts }
        }
        Verb::Config {
            quiet,
            no_quiet,
            weekly_cap,
            json,
        } => WakeAction::Config {
            quiet,
            no_quiet,
            weekly_cap,
            json,
        },
        Verb::Run {
            once,
            dry_run,
            json,
        } => {
            if json && !once {
                return Err(usage_error("--json needs --once"));
            }
            WakeAction::Run {
                once,
                dry_run,
                json,
            }
        }
        Verb::Install => WakeAction::Install,
        Verb::Uninstall => WakeAction::Uninstall,
    };
    Ok(Invocation {
        debug: args.debug,
        command: Command::Wake(action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(argv: &[&str]) -> Result<WakeAction, Error> {
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        parse(&args).map(|inv| match inv.command {
            Command::Wake(action) => action,
            other => panic!("unexpected {other:?}"),
        })
    }

    #[test]
    fn definition_is_consistent() {
        WakeArgs::command().debug_assert();
    }

    #[test]
    fn bare_wake_is_status() {
        assert_eq!(parse_args(&[]).unwrap(), WakeAction::Status { json: false });
    }

    #[test]
    fn on_takes_accounts_or_all() {
        assert_eq!(
            parse_args(&["on", "1", "work"]).unwrap(),
            WakeAction::On {
                accounts: vec!["1".into(), "work".into()]
            }
        );
        assert!(parse_args(&["on", "all"]).is_ok());
        assert!(parse_args(&["off", "all", "2"]).is_err());
        assert!(parse_args(&["on"]).is_err());
    }

    #[test]
    fn config_flags() {
        assert_eq!(
            parse_args(&["config", "--quiet", "01:00-06:00", "--weekly-cap", "85"]).unwrap(),
            WakeAction::Config {
                quiet: Some("01:00-06:00".into()),
                no_quiet: false,
                weekly_cap: Some(85.0),
                json: false
            }
        );
        assert!(parse_args(&["config", "--quiet", "01:00-02:00", "--no-quiet"]).is_err());
    }

    #[test]
    fn run_json_needs_once() {
        assert!(parse_args(&["run", "--json"]).is_err());
        assert!(parse_args(&["run", "--once", "--json", "--dry-run"]).is_ok());
    }
}
