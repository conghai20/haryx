//! `haryx ai sessions`: every live Claude Code session, by account.

use clap::{Parser, Subcommand};

use super::help::{self, p, sub_bin};
use super::{Command, Error, Invocation};

#[derive(Parser, Debug)]
#[command(
    name = "sessions",
    bin_name = sub_bin("sessions"),
    about = p("List the Claude Code sessions running on this machine and the account each runs as. \
               Read from Claude Code's own session records, so it works on every OS."),
    after_help = p("Examples:\n  {ai} sessions\n  {ai} sessions --json\n  \
                    {ai} sessions continue 4242 --to best"),
    help_template = help::HELP_TEMPLATE,
)]
struct SessionsArgs {
    #[command(subcommand)]
    action: Option<Action>,
    /// Print JSON
    #[arg(long, global = true)]
    json: bool,
    /// Enable debug logging
    #[arg(long, global = true)]
    debug: bool,
}

#[derive(Subcommand, Debug)]
enum Action {
    /// End a running session and continue the same conversation on another account, in this
    /// terminal and the session's folder
    Continue {
        /// The session's pid, id, or a unique id prefix (8+ characters)
        session: String,
        /// The account to continue on: a number, an email, or 'best' (the most headroom)
        #[arg(long, value_name = "NUM|EMAIL|best", default_value = "best")]
        to: String,
        /// Prepare (end the old session, make the conversation reachable) and print the command
        /// instead of running it
        #[arg(long)]
        no_launch: bool,
    },
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args = SessionsArgs::try_parse_from(
        std::iter::once(haryx_core::PROG.to_owned()).chain(rest.iter().cloned()),
    )?;
    let command = match args.action {
        None => Command::Sessions { json: args.json },
        Some(Action::Continue {
            session,
            to,
            no_launch,
        }) => Command::SessionsContinue {
            session,
            to,
            json: args.json,
            no_launch: no_launch || args.json,
        },
    };
    Ok(Invocation {
        debug: args.debug,
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_consistent() {
        SessionsArgs::command().debug_assert();
    }

    #[test]
    fn json_flag_is_optional() {
        let parse_json = |argv: &[&str]| {
            let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
            match parse(&args).unwrap().command {
                Command::Sessions { json } => json,
                other => panic!("unexpected {other:?}"),
            }
        };
        assert!(!parse_json(&[]));
        assert!(parse_json(&["--json"]));
        assert!(parse(&["extra".to_owned()]).is_err());
    }

    #[test]
    fn continue_takes_a_session_and_a_target() {
        let parsed = |argv: &[&str]| {
            let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
            parse(&args).unwrap().command
        };
        match parsed(&["continue", "4242"]) {
            Command::SessionsContinue {
                session,
                to,
                json,
                no_launch,
            } => {
                assert_eq!(
                    (session.as_str(), to.as_str(), json, no_launch),
                    ("4242", "best", false, false)
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        match parsed(&["continue", "abc12345", "--to", "3", "--json"]) {
            Command::SessionsContinue {
                to,
                json,
                no_launch,
                ..
            } => {
                assert_eq!(to, "3");
                assert!(json && no_launch, "--json never launches");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse(&["continue".to_owned()]).is_err());
    }
}
