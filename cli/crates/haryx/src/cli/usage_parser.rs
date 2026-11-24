//! `haryx ai usage`: usage history from Claude Code's transcripts (docs/usage-history-plan.md).

use clap::{Parser, Subcommand};

use super::help::{self, p, sub_bin};
use super::{Command, Error, Invocation};

#[derive(Parser, Debug)]
#[command(
    name = "usage",
    bin_name = sub_bin("usage"),
    about = p("Usage history: tokens and what they would cost at API prices, by account, project, \
               session, model or day, from Claude Code's own transcripts. Kept on this machine."),
    after_help = p("Examples:\n  {ai} usage\n  {ai} usage report --since 30d --by project\n  \
                    {ai} usage quota --account 2 --json\n  {ai} usage summary\n  \
                    {ai} usage index --import all"),
    help_template = help::HELP_TEMPLATE,
)]
struct UsageArgs {
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
    /// Read new transcript lines into the history
    Index {
        /// Forget what was read and read every transcript again
        #[arg(long)]
        rebuild: bool,
        /// Answer the first-run question: read past transcripts (all) or start now (none)
        #[arg(long, value_parser = ["all", "none"])]
        import: Option<String>,
    },
    /// Tokens and cost grouped by account, project, session, model or day
    Report {
        /// How far back: 24h, 7d, 30d, all, or a date (YYYY-MM-DD)
        #[arg(long, default_value = "7d")]
        since: String,
        /// Group by
        #[arg(long, default_value = "account", value_parser = ["account", "project", "session", "model", "day"])]
        by: String,
        /// Only this account (a number)
        #[arg(long)]
        account: Option<i64>,
        /// Only folders under this path
        #[arg(long)]
        project: Option<String>,
    },
    /// The 5-hour and weekly readings over time
    Quota {
        #[arg(long, default_value = "7d")]
        since: String,
        #[arg(long)]
        account: Option<i64>,
    },
    /// This week by account: tokens, cost, times it ran out, its biggest project
    Summary {
        #[arg(long, default_value = "7d")]
        since: String,
    },
}

/// What `ai usage` was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageAction {
    Index {
        rebuild: bool,
        import: Option<String>,
    },
    Report {
        since: String,
        by: String,
        account: Option<i64>,
        project: Option<String>,
    },
    Quota {
        since: String,
        account: Option<i64>,
    },
    Summary {
        since: String,
    },
}

pub(super) fn parse(rest: &[String]) -> Result<Invocation, Error> {
    let args = UsageArgs::try_parse_from(
        std::iter::once(haryx_core::PROG.to_owned()).chain(rest.iter().cloned()),
    )?;
    let action = match args.action {
        None => UsageAction::Report {
            since: "7d".into(),
            by: "account".into(),
            account: None,
            project: None,
        },
        Some(Action::Index { rebuild, import }) => UsageAction::Index { rebuild, import },
        Some(Action::Report {
            since,
            by,
            account,
            project,
        }) => UsageAction::Report {
            since,
            by,
            account,
            project,
        },
        Some(Action::Quota { since, account }) => UsageAction::Quota { since, account },
        Some(Action::Summary { since }) => UsageAction::Summary { since },
    };
    Ok(Invocation {
        debug: args.debug,
        command: Command::Usage {
            action,
            json: args.json,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parsed(argv: &[&str]) -> (UsageAction, bool) {
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        match parse(&args).unwrap().command {
            Command::Usage { action, json } => (action, json),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn definition_is_consistent() {
        UsageArgs::command().debug_assert();
    }

    #[test]
    fn bare_usage_is_the_weekly_report_by_account() {
        assert_eq!(
            parsed(&[]).0,
            UsageAction::Report {
                since: "7d".into(),
                by: "account".into(),
                account: None,
                project: None
            }
        );
    }

    #[test]
    fn subcommands_and_flags() {
        assert_eq!(
            parsed(&["report", "--since", "30d", "--by", "project", "--json"]),
            (
                UsageAction::Report {
                    since: "30d".into(),
                    by: "project".into(),
                    account: None,
                    project: None
                },
                true
            )
        );
        assert_eq!(
            parsed(&["index", "--import", "all"]).0,
            UsageAction::Index {
                rebuild: false,
                import: Some("all".into())
            }
        );
        assert_eq!(
            parsed(&["quota", "--account", "2"]).0,
            UsageAction::Quota {
                since: "7d".into(),
                account: Some(2)
            }
        );
        let bad: Vec<String> = ["report", "--by", "color"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert!(parse(&bad).is_err());
    }
}
