//! The top level of `haryx`: the section overview (`haryx --help`) and the error for an `ai`-only
//! command typed without its section.

use clap::error::ErrorKind;
use haryx_core::{AI_PROG, PROG};

use super::{Error, help};

/// The top-level parser: no arguments of its own, only the overview help and the version.
fn command() -> clap::Command {
    clap::Command::new(PROG)
        .bin_name(PROG)
        .version(help::version())
        .override_usage(help::top_usage())
        .about(help::top_description())
        .help_template(help::HELP_TEMPLATE)
        .disable_help_subcommand(true)
}

/// `haryx --help` / `haryx help`: the overview, printed on stdout with exit code 0.
pub(super) fn help() -> Error {
    command()
        .try_get_matches_from([PROG, "--help"])
        .err()
        .unwrap_or_else(|| command().error(ErrorKind::DisplayHelp, help::top_description()))
}

/// Bare `haryx` with no terminal (a script): a usage error pointing at the overview (exit code 2).
pub(super) fn no_command() -> Error {
    command().error(
        ErrorKind::MissingRequiredArgument,
        format!("no command given — try '{PROG} help'"),
    )
}

/// `haryx login`: a usage error naming the `ai` form (exit code 2).
pub(super) fn moved_to_ai(sub: &str) -> Error {
    command().error(
        ErrorKind::InvalidSubcommand,
        format!("'{PROG} {sub}' is reserved for HaryX itself — did you mean '{AI_PROG} {sub}'?"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_command_definition_is_consistent() {
        command().debug_assert();
    }

    #[test]
    fn help_is_a_display_help_request() {
        assert_eq!(help().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn moved_command_names_the_ai_form() {
        let err = moved_to_ai("login");
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
        assert!(err.to_string().contains("haryx ai login"));
    }
}
