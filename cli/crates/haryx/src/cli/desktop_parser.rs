//! `haryx ai desktop`: one Claude Desktop data folder per account (plan: `docs/desktop-follow-plan.md`).

use clap::{Parser, Subcommand};

use super::help::{self, p, sub_bin};
use super::{Command, Error, Invocation};

/// `desktop` verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopAction {
    Status { json: bool },
    Path { account: String },
}

#[derive(Parser, Debug)]
#[command(
    name = "desktop",
    bin_name = sub_bin("desktop"),
    about = p("Show each account's Claude Desktop data folder: whether it exists and which account \
               it is signed into. The macOS app opens Claude Desktop on these folders."),
    after_help = desktop_epilog(),
    help_template = help::HELP_TEMPLATE,
    disable_help_subcommand = true,
    subcommand_value_name = "{status,path}",
    subcommand_help_heading = "Actions",
)]
struct DesktopArgs {
    /// Enable debug logging
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    action: Option<Verb>,
}

#[derive(Subcommand, Debug)]
enum Verb {
    /// Show each account's folder and sign-in (the default)
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Print the folder Claude Desktop should open with for one account
    Path {
        #[arg(value_name = "NUM|EMAIL|ALIAS")]
        account: String,
    },
}

fn desktop_epilog() -> String {
    p(
        "Folders are never created, moved, copied or deleted by this command.

Examples:
  {ai} desktop status --json
  {ai} desktop path work",
    )
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args = DesktopArgs::try_parse_from(
        std::iter::once(haryx_core::PROG.to_owned()).chain(rest.iter().cloned()),
    )?;
    let action = match args.action.unwrap_or(Verb::Status { json: false }) {
        Verb::Status { json } => DesktopAction::Status { json },
        Verb::Path { account } => DesktopAction::Path { account },
    };
    Ok(Invocation {
        debug: args.debug,
        command: Command::Desktop(action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse_args(argv: &[&str]) -> Result<DesktopAction, Error> {
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        parse(&args).map(|inv| match inv.command {
            Command::Desktop(action) => action,
            other => panic!("unexpected {other:?}"),
        })
    }

    #[test]
    fn definition_is_consistent() {
        DesktopArgs::command().debug_assert();
    }

    #[test]
    fn bare_desktop_is_status() {
        assert_eq!(
            parse_args(&[]).unwrap(),
            DesktopAction::Status { json: false }
        );
        assert_eq!(
            parse_args(&["status", "--json"]).unwrap(),
            DesktopAction::Status { json: true }
        );
    }

    #[test]
    fn path_needs_one_account() {
        assert_eq!(
            parse_args(&["path", "work"]).unwrap(),
            DesktopAction::Path {
                account: "work".into()
            }
        );
        assert!(parse_args(&["path"]).is_err());
        assert!(parse_args(&["path", "1", "2"]).is_err());
    }
}
