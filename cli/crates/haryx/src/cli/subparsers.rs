//! The commands upstream `main()` dispatches before building its main parser, each with its own
//! argparse parser: `run`, `auto`, `config`, `map`, `unmap`, `unclaimed`, `alias`, `threshold`,
//! `priority`, `swap`, `move`.

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};
use haryx_core::PROG;

use super::help::{self, p, sub_bin};
use super::{
    AccountSetting, AutoOptions, AutoStrategy, Command, ConfigAction, Error, Invocation,
    LoginOptions, RunOptions,
};

/// Parse `rest` with the pre-dispatched parser named by `first`, or `None` when `first` is not one.
pub(super) fn parse(first: &str, rest: &[String]) -> Option<Result<Invocation, Error>> {
    let parsed = match first {
        "run" => parse_run(rest),
        "auto" => run_parser::<AutoArgs>(rest).map(AutoArgs::into_invocation),
        "config" => run_parser::<ConfigArgs>(rest).and_then(ConfigArgs::into_invocation),
        "map" => run_parser::<MapArgs>(rest).map(|a| invocation(a.debug, a.into_command())),
        "unmap" => run_parser::<UnmapArgs>(rest)
            .map(|a| invocation(a.debug, Command::Unmap { path: a.path })),
        "unclaimed" => run_parser::<UnclaimedArgs>(rest).map(|a| {
            // Upstream tests `if args.purge:`, so an empty ID lists.
            let purge = a.purge.filter(|id| !id.is_empty());
            invocation(a.debug, Command::Unclaimed { purge })
        }),
        "alias" => run_parser::<AliasArgs>(rest).and_then(AliasArgs::into_invocation),
        "threshold" => run_parser::<ThresholdArgs>(rest).and_then(ThresholdArgs::into_invocation),
        "priority" => run_parser::<PriorityArgs>(rest).and_then(PriorityArgs::into_invocation),
        "swap" => run_parser::<SwapArgs>(rest).map(|a| {
            invocation(
                a.debug,
                Command::Swap {
                    first: a.first,
                    second: a.second,
                },
            )
        }),
        "login" => run_parser::<LoginArgs>(rest).and_then(LoginArgs::into_invocation),
        "move" => run_parser::<MoveArgs>(rest).map(|a| {
            invocation(
                a.debug,
                Command::Move {
                    account: a.account,
                    slot: a.slot,
                },
            )
        }),
        _ => return None,
    };
    Some(parsed)
}

fn run_parser<P: Parser>(rest: &[String]) -> Result<P, Error> {
    P::try_parse_from(std::iter::once(PROG.to_owned()).chain(rest.iter().cloned()))
}

fn invocation(debug: bool, command: Command) -> Invocation {
    Invocation { debug, command }
}

fn usage_error<P: CommandFactory>(message: &str) -> Error {
    P::command().error(ErrorKind::ArgumentConflict, message)
}

// ---------------------------------------------------------------------------------------------- run

/// `haryx run NUM|EMAIL [--no-share] [-- <claude args>]`. Everything after the first `--` goes to
/// claude verbatim and is split off before parsing.
fn parse_run(rest: &[String]) -> Result<Invocation, Error> {
    let (head, tail) = match rest.iter().position(|arg| arg == "--") {
        Some(split) => (&rest[..split], &rest[split + 1..]),
        None => (rest, &[][..]),
    };
    let args = run_parser::<RunArgs>(head)?;
    let share_history = match (args.share_history, args.no_share_history) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    let options = RunOptions {
        account: args.account,
        no_share: args.no_share,
        share_history,
        require_session: args.require_session,
        claude_args: tail.to_vec(),
    };
    Ok(invocation(args.debug, Command::Run(options)))
}

#[derive(Parser, Debug)]
#[command(
    name = "run",
    bin_name = sub_bin("run"),
    about = "[EXPERIMENTAL] Launch Claude Code as a stored account in this terminal only (the \
             default login and other terminals are unaffected).",
    after_help = help::run_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct RunArgs {
    #[arg(
        value_name = "NUM|EMAIL",
        help = p("Account to run (number or email). Omit to use the current directory's mapping \
                  (see `{prog} ai map`).")
    )]
    account: Option<String>,

    /// Don't share settings/keybindings/CLAUDE.md/skills/commands/agents/rules from ~/.claude into
    /// the session profile (and remove previously shared items). Chat history is not affected: see
    /// --no-share-history
    #[arg(long)]
    no_share: bool,

    #[arg(
        long,
        overrides_with = "no_share_history",
        help = p("Share conversation history (projects/ and history.jsonl) from ~/.claude into the \
                  session profile, so every account sees one unified history. This is the \
                  default; history the profile already accumulated is merged into ~/.claude \
                  first. --no-share-history keeps this account's history separate for this \
                  launch, `{prog} ai config set run.shareHistory false` for every launch.")
    )]
    share_history: bool,

    /// Keep this account's chat history separate for this launch
    #[arg(long, overrides_with = "share_history")]
    no_share_history: bool,

    /// Refuse to launch when the account is already the active default login, instead of running
    /// plain claude on that login (which a later switch could pull out from under the session)
    #[arg(long)]
    require_session: bool,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

// --------------------------------------------------------------------------------------------- auto

#[derive(Parser, Debug)]
#[command(
    name = "auto",
    bin_name = sub_bin("auto"),
    about = "Move to another of your own accounts when the active one reaches your switch threshold \
             (5h/7d usage). Runs a foreground polling loop; use --once for a single tick \
             (cron-friendly). Plan limits are Anthropic's: use only your own accounts.",
    after_help = help::auto_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
    allow_negative_numbers = true,
)]
struct AutoArgs {
    /// Evaluate once, maybe switch, and exit (exit code = outcome)
    #[arg(long)]
    once: bool,

    /// Emit one machine-readable JSON event per line on stdout
    #[arg(long)]
    json: bool,

    /// Poll interval in loop mode (min 15; default 60)
    #[arg(long, value_name = "SECONDS")]
    interval: Option<f64>,

    /// Switch when the active account's 5h window (or any window, without a weekly line) reaches
    /// this utilization (50-99.9; default 90)
    #[arg(long, value_name = "PCT")]
    threshold: Option<f64>,

    #[arg(
        long,
        value_name = "PCT",
        help = p("Separate switch line for the 7-day and per-model weekly windows (50-99.9; \
                  default: the threshold). A flag cannot unset a configured value; use \
                  '{prog} ai config unset'")
    )]
    weekly_threshold: Option<f64>,

    /// Minimum time between proactive switches (default 300)
    #[arg(long, value_name = "SECONDS")]
    cooldown: Option<f64>,

    /// Also switch when a per-model weekly limit is hit, not just the account-wide 5h/7d windows.
    /// One name or a comma-separated list (e.g. Fable, Opus, Sonnet, Haiku, or 'Fable,Opus'), or
    /// 'all' for every per-model window an account reports
    #[arg(long, value_name = "NAMES")]
    model: Option<String>,

    /// Allow switching onto managed API-key accounts as a last resort (they bill per token;
    /// default: excluded)
    #[arg(long, overrides_with = "no_include_api_key_accounts")]
    include_api_key_accounts: bool,

    /// Keep managed API-key accounts out of auto-switch targets
    #[arg(long, overrides_with = "include_api_key_accounts")]
    no_include_api_key_accounts: bool,

    /// Target selection: 'best' (most quota left; default), 'weekly-first' (switch at the threshold
    /// like best, onto the account whose weekly window resets soonest), 'consume-first'
    /// (proactively use the account whose weekly window resets soonest), or
    /// 'priority-consume-first' (highest account priority, then soonest weekly reset; only targets
    /// below all switch lines)
    #[arg(long, value_enum)]
    strategy: Option<AutoStrategy>,

    /// Evaluate and report, but never switch or write state
    #[arg(long)]
    dry_run: bool,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

impl AutoArgs {
    fn into_invocation(self) -> Invocation {
        let include_api_key_accounts = match (
            self.include_api_key_accounts,
            self.no_include_api_key_accounts,
        ) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        };
        let options = AutoOptions {
            once: self.once,
            json: self.json,
            interval: self.interval,
            threshold: self.threshold,
            weekly_threshold: self.weekly_threshold,
            cooldown: self.cooldown,
            model: self.model,
            include_api_key_accounts,
            strategy: self.strategy,
            dry_run: self.dry_run,
        };
        invocation(self.debug, Command::Auto(options))
    }
}

// ------------------------------------------------------------------------------------------- config

#[derive(Parser, Debug)]
#[command(
    name = "config",
    bin_name = sub_bin("config"),
    about = p("Read and edit {prog} settings (settings.json in the backup root)."),
    after_help = help::config_epilog(),
    help_template = help::HELP_TEMPLATE,
    subcommand_value_name = "{list,get,set,unset,path}",
    subcommand_help_heading = "Actions",
    disable_help_subcommand = true,
    args_override_self = true,
    infer_long_args = true,
)]
struct ConfigArgs {
    /// Emit machine-readable JSON to stdout (with list or get)
    #[arg(long, global = true)]
    json: bool,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    #[command(subcommand)]
    action: Option<ConfigVerb>,
}

#[derive(Subcommand, Debug)]
enum ConfigVerb {
    /// Show all effective settings (the default)
    List,
    /// Print one setting's effective value
    Get {
        /// Dotted key, e.g. autoswitch.threshold
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Validate and persist one setting
    #[command(allow_negative_numbers = true)]
    Set {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "VALUE")]
        value: String,
    },
    /// Remove one setting (revert to the default)
    Unset {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Print the settings.json location
    Path,
}

impl ConfigArgs {
    fn into_invocation(self) -> Result<Invocation, Error> {
        let action = match self.action {
            None | Some(ConfigVerb::List) => ConfigAction::List,
            Some(ConfigVerb::Get { key }) => ConfigAction::Get { key },
            Some(ConfigVerb::Set { key, value }) => ConfigAction::Set { key, value },
            Some(ConfigVerb::Unset { key }) => ConfigAction::Unset { key },
            Some(ConfigVerb::Path) => ConfigAction::Path,
        };
        let reads = matches!(action, ConfigAction::List | ConfigAction::Get { .. });
        if self.json && !reads {
            return Err(usage_error::<Self>(
                "--json can only be used with list or get",
            ));
        }
        Ok(invocation(
            self.debug,
            Command::Config {
                json: self.json,
                action,
            },
        ))
    }
}

// ------------------------------------------------------------------------------------- map / unmap

#[derive(Parser, Debug)]
#[command(
    name = "map",
    bin_name = sub_bin("map"),
    about = p("Map a stored account to a directory so `{prog} ai run` (with no account) auto-launches \
               it there. With no arguments, lists all mappings."),
    after_help = help::map_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct MapArgs {
    /// Account to map (number or email). Omit to list mappings.
    #[arg(value_name = "NUM|EMAIL")]
    account: Option<String>,

    /// Directory to map (default: current directory)
    #[arg(value_name = "PATH")]
    path: Option<String>,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

impl MapArgs {
    fn into_command(self) -> Command {
        Command::Map {
            account: self.account,
            path: self.path,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "unmap",
    bin_name = sub_bin("unmap"),
    about = "Remove a directory → account mapping (default: current directory).",
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct UnmapArgs {
    /// Directory to unmap (default: current directory)
    #[arg(value_name = "PATH")]
    path: Option<String>,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

// ---------------------------------------------------------------------------------------- unclaimed

#[derive(Parser, Debug)]
#[command(
    name = "unclaimed",
    bin_name = sub_bin("unclaimed"),
    about = p("List stashed credential entries, or purge one by id. Purging deletes the bytes — \
               recovery is /login + `{prog} ai add`."),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct UnclaimedArgs {
    /// Delete this entry's bytes and manifest row
    #[arg(long, value_name = "ID")]
    purge: Option<String>,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

// -------------------------------------------------------------------------------------- swap / move

#[derive(Parser, Debug)]
#[command(
    name = "swap",
    bin_name = sub_bin("swap"),
    about = p("Exchange two accounts' slot numbers, so they trade places in `{prog} ai list` and as \
               numeric targets. Aliases, backups, and session history move with their account."),
    after_help = help::swap_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct SwapArgs {
    /// One account
    #[arg(value_name = "NUM|EMAIL|ALIAS")]
    first: String,

    /// The other account
    #[arg(value_name = "NUM|EMAIL|ALIAS")]
    second: String,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

#[derive(Parser, Debug)]
#[command(
    name = "move",
    bin_name = sub_bin("move"),
    about = "Assign an account to a slot number. An empty slot relocates the account there and \
             frees its old slot; an occupied slot swaps the two. Aliases, backups, and session \
             history move with the account.",
    after_help = help::move_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
    allow_negative_numbers = true,
)]
struct MoveArgs {
    /// Account to move
    #[arg(value_name = "NUM|EMAIL|ALIAS")]
    account: String,

    /// Destination slot number
    #[arg(value_name = "SLOT")]
    slot: String,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

// -------------------------------------------------------------------------------------------- login

#[derive(Parser, Debug)]
#[command(
    name = "login",
    bin_name = sub_bin("login"),
    about = p("Sign in through Claude Code's own login and store the result. With an account, signs \
               that account in again (the fix for \"re-login needed\"); without one, adds a new \
               account. The active login is only touched when the account given is the active one."),
    after_help = help::login_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct LoginArgs {
    /// Account to sign in again (omit to add a new account)
    #[arg(value_name = "NUM|EMAIL|ALIAS")]
    account: Option<String>,

    /// Slot for a new account (asks before replacing another account)
    #[arg(long, value_name = "NUM")]
    slot: Option<i64>,

    /// Short display alias for a new account
    #[arg(long, value_name = "NAME")]
    alias: Option<String>,

    /// Sign in with your organization's single sign-on
    #[arg(long)]
    sso: bool,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

impl LoginArgs {
    fn into_invocation(self) -> Result<Invocation, Error> {
        if self.account.is_some() && (self.slot.is_some() || self.alias.is_some()) {
            return Err(usage_error::<Self>(
                "--slot and --alias are only for a new account (omit the account)",
            ));
        }
        let options = LoginOptions {
            account: self.account,
            slot: self.slot,
            alias: self.alias,
            sso: self.sso,
        };
        Ok(invocation(self.debug, Command::Login(options)))
    }
}

// ---------------------------------------------------------------------- alias / threshold / priority

/// The shared list / set / unset rules of `alias`, `threshold` and `priority`.
struct SettingRules {
    unset_takes_no_value: &'static str,
    unset_needs_account: &'static str,
    set_needs_value: &'static str,
}

/// Whether the value argument was given, by each upstream parser's own test: `blocks_unset` for the
/// `--unset` check and `satisfies_set` for the "value is required" check (truthiness or
/// `is not None`, which differ for an empty string).
struct ValueGiven {
    blocks_unset: bool,
    satisfies_set: bool,
}

fn account_setting<P: CommandFactory>(
    rules: &SettingRules,
    account: Option<String>,
    value: Option<String>,
    given: ValueGiven,
    unset: bool,
) -> Result<AccountSetting, Error> {
    if unset && given.blocks_unset {
        return Err(usage_error::<P>(rules.unset_takes_no_value));
    }
    match account {
        None if unset => Err(usage_error::<P>(rules.unset_needs_account)),
        None => Ok(AccountSetting::List),
        Some(account) if unset => Ok(AccountSetting::Unset { account }),
        Some(_) if !given.satisfies_set => Err(usage_error::<P>(rules.set_needs_value)),
        Some(account) => Ok(AccountSetting::Set {
            account,
            value: value.unwrap_or_default(),
        }),
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "alias",
    bin_name = sub_bin("alias"),
    about = "Set, remove, or list a short display alias for an account. Once set, the alias can be \
             used anywhere an account number or email is accepted (switch, remove, run, map).",
    after_help = help::alias_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
)]
struct AliasArgs {
    /// Account to alias (number or email). Omit to list aliases.
    #[arg(value_name = "NUM|EMAIL")]
    account: Option<String>,

    /// Alias to set (letters, digits, ., -, _; not purely numeric).
    #[arg(value_name = "NAME")]
    alias_name: Option<String>,

    /// Remove the account's alias
    #[arg(long)]
    unset: bool,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

impl AliasArgs {
    fn into_invocation(self) -> Result<Invocation, Error> {
        let rules = SettingRules {
            unset_takes_no_value: "--unset does not take a NAME argument",
            unset_needs_account: "NUM|EMAIL is required with --unset",
            set_needs_value: "NAME is required (or pass --unset to remove the alias)",
        };
        // Upstream tests `args.alias_name` for truth: an empty NAME counts as none.
        let truthy = self
            .alias_name
            .as_deref()
            .is_some_and(|name| !name.is_empty());
        let given = ValueGiven {
            blocks_unset: truthy,
            satisfies_set: truthy,
        };
        let setting =
            account_setting::<Self>(&rules, self.account, self.alias_name, given, self.unset)?;
        Ok(invocation(self.debug, Command::Alias(setting)))
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "threshold",
    bin_name = sub_bin("threshold"),
    about = "Set, remove, or list a per-account switch-away threshold. The per-account value \
             replaces autoswitch.threshold for that account only, so one shared account can \
             reserve headroom without capping the whole fleet.",
    after_help = help::threshold_epilog(),
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
    allow_negative_numbers = true,
)]
struct ThresholdArgs {
    /// Account to set (number, email, or alias). Omit to list.
    #[arg(value_name = "NUM|EMAIL")]
    account: Option<String>,

    #[arg(value_name = "PCT", help = help::threshold_pct_help())]
    threshold: Option<String>,

    /// Remove the account's override
    #[arg(long)]
    unset: bool,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

impl ThresholdArgs {
    fn into_invocation(self) -> Result<Invocation, Error> {
        let rules = SettingRules {
            unset_takes_no_value: "--unset does not take a PCT argument",
            unset_needs_account: "NUM|EMAIL is required with --unset",
            set_needs_value: "PCT is required (or pass --unset to remove the override)",
        };
        // Upstream: `--unset` rejects a truthy PCT; setting needs `PCT is not None`.
        let given = ValueGiven {
            blocks_unset: self.threshold.as_deref().is_some_and(|pct| !pct.is_empty()),
            satisfies_set: self.threshold.is_some(),
        };
        let setting =
            account_setting::<Self>(&rules, self.account, self.threshold, given, self.unset)?;
        Ok(invocation(self.debug, Command::Threshold(setting)))
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "priority",
    bin_name = sub_bin("priority"),
    about = "Set or list account priority tiers for the optional priority-consume-first strategy. \
             Higher numbers are used first; accounts in the same tier are ordered by soonest \
             weekly reset.",
    help_template = help::HELP_TEMPLATE,
    args_override_self = true,
    infer_long_args = true,
    allow_negative_numbers = true,
)]
struct PriorityArgs {
    #[arg(value_name = "NUM|EMAIL|ALIAS")]
    account: Option<String>,

    #[arg(value_name = "TIER", help = help::priority_tier_help())]
    priority: Option<String>,

    /// Return to priority 0
    #[arg(long)]
    unset: bool,

    #[arg(long)]
    debug: bool,
}

impl PriorityArgs {
    fn into_invocation(self) -> Result<Invocation, Error> {
        let rules = SettingRules {
            unset_takes_no_value: "--unset does not take a TIER argument",
            unset_needs_account: "NUM|EMAIL|ALIAS is required with --unset",
            set_needs_value: "TIER is required (or pass --unset)",
        };
        let given = ValueGiven {
            blocks_unset: self.priority.is_some(),
            satisfies_set: self.priority.is_some(),
        };
        let setting =
            account_setting::<Self>(&rules, self.account, self.priority, given, self.unset)?;
        Ok(invocation(self.debug, Command::Priority(setting)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definitions_are_consistent() {
        RunArgs::command().debug_assert();
        AutoArgs::command().debug_assert();
        ConfigArgs::command().debug_assert();
        MapArgs::command().debug_assert();
        UnmapArgs::command().debug_assert();
        UnclaimedArgs::command().debug_assert();
        SwapArgs::command().debug_assert();
        MoveArgs::command().debug_assert();
        AliasArgs::command().debug_assert();
        ThresholdArgs::command().debug_assert();
        PriorityArgs::command().debug_assert();
    }
}
