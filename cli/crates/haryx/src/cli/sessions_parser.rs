//! `haryx ai sessions`: every live Claude Code session, by account.

use clap::Parser;

use super::help::{self, p, sub_bin};
use super::{Command, Error, Invocation};

#[derive(Parser, Debug)]
#[command(
    name = "sessions",
    bin_name = sub_bin("sessions"),
    about = p("List the Claude Code sessions running on this machine and the account each runs as. \
               Read from Claude Code's own session records, so it works on every OS."),
    after_help = p("Examples:\n  {ai} sessions\n  {ai} sessions --json"),
    help_template = help::HELP_TEMPLATE,
)]
struct SessionsArgs {
    /// Print JSON
    #[arg(long)]
    json: bool,
    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args = SessionsArgs::try_parse_from(
        std::iter::once(haryx_core::PROG.to_owned()).chain(rest.iter().cloned()),
    )?;
    Ok(Invocation {
        debug: args.debug,
        command: Command::Sessions { json: args.json },
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
}
