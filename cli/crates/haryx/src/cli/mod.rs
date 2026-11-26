//! The `haryx` command line: a port of the argument surface of `claude_swap/cli.py` (1.2.0b1).
//!
//! Upstream has two interfaces, and both are kept:
//! - the long-standing flags (`--list`, `--switch`, `--switch-to X`, `--add-account`, ...), parsed by
//!   the main parser ([`main_parser`]), with one mutually exclusive group of actions;
//! - the memorable subcommands (`list`, `switch 2`, `add`, ...), rewritten into those flags by
//!   [`translate_subcommand`] before the main parser runs.
//!
//! A few commands are dispatched *before* the main parser, each with a parser of its own
//! ([`subparsers`]): `run`, `auto`, `config`, `map`, `unmap`, `unclaimed`, `alias`, `threshold`,
//! `priority`, `swap`, `move`, `login`. As upstream, they must be the first argument (`haryx --debug run 2` is a
//! usage error; `haryx run 2 --debug` works).
//!
//! Dropped from upstream: `menubar` and its flags (`--install-service`, `--uninstall-service`,
//! `--service-status`, `--foreground`, `--auto-switch`, `--no-auto-switch`). All of them drive the
//! Python menu bar's LaunchAgent; the HaryX macOS app replaces that menu bar.
//!
//! Added: `import --from-cswap`, which copies accounts from an existing cswap store.
//!
//! Sections: every command above lives under `haryx ai` (`haryx ai list`, `haryx ai login 3`). The
//! upstream spellings without `ai` stay as undocumented aliases, except `login`: top-level `haryx
//! login` (with `logout` and `whoami`) is the HaryX account itself ([`account_parser`]). `haryx
//! --help` lists the sections.

mod account_parser;
mod desktop_parser;
pub mod help;
mod main_parser;
mod profile_parser;
mod sessions_parser;
mod subparsers;
mod sync_parser;
mod usage_parser;
pub use usage_parser::UsageAction;
mod top;
mod wake_parser;

pub use account_parser::AccountAction;
pub use clap::Error;
pub use desktop_parser::DesktopAction;
pub use profile_parser::{PartFlags, PluginsChoice, ProfileAction, SecretsChoice};
pub use sync_parser::SyncAction;
pub use wake_parser::WakeAction;

/// One parsed invocation: the action plus the options every parser shares.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    /// `--debug`: debug-level logging on stderr.
    pub debug: bool,
    pub command: Command,
}

/// Every action `haryx` can take, with its options. One variant per upstream dispatch branch.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `tui` / `--tui`, or bare `haryx` with stdin and stdout on a terminal.
    Tui,
    /// `watch` / `--watch`: the dashboard opened on its live watch page.
    Watch,
    /// `list` / `ls` / `--list`.
    List { token_status: bool, json: bool },
    /// `status` / `--status`.
    Status { json: bool },
    /// `doctor`: whether Claude Code is installed and signed in, with the fix for each problem.
    Doctor { json: bool },
    /// Bare `switch` / `--switch`: rotate, or pick by quota with `--strategy`.
    Switch {
        strategy: Option<SwitchStrategy>,
        /// `--model NAMES` (only with a strategy); `None` falls back to `autoswitch.model`.
        model: Option<String>,
        json: bool,
    },
    /// `switch <num|email>` / `--switch-to X`.
    SwitchTo {
        target: String,
        json: bool,
        force: bool,
    },
    /// `add` / `--add-account`.
    Add {
        slot: Option<i64>,
        alias: Option<String>,
    },
    /// `add-token [TOKEN|-]` / `--add-token`. `token: None` (no value, or an empty one) prompts.
    AddToken {
        token: Option<String>,
        email: Option<String>,
        slot: Option<i64>,
    },
    /// `remove` / `rm` / `--remove-account`.
    Remove { account: String },
    /// `disable` / `--disable-account`.
    Disable { account: String },
    /// `enable` / `--enable-account`.
    Enable { account: String },
    /// `export PATH` / `--export PATH`.
    Export {
        path: String,
        account: Option<String>,
        full: bool,
    },
    /// `import PATH` / `--import PATH`, or `import --from-cswap`.
    Import { source: ImportSource, force: bool },
    /// `import-usage PATH` / `--import-usage PATH`.
    ImportUsage {
        path: String,
        hold_seconds: Option<f64>,
    },
    /// `purge` / `--purge`.
    Purge,
    /// `upgrade` / `update` / `--upgrade`, `upgrade --check [--json] [--refresh]`, or
    /// `upgrade --download-app DIR [--json]`.
    Upgrade {
        check: bool,
        json: bool,
        refresh: bool,
        download_app: Option<String>,
    },
    /// `run [NUM|EMAIL] [-- claude args]`.
    Run(RunOptions),
    /// `auto [...]`.
    Auto(AutoOptions),
    /// `config [--json] [list|get|set|unset|path]`.
    Config { json: bool, action: ConfigAction },
    /// `map [NUM|EMAIL] [PATH]`. No account lists the mappings.
    Map {
        account: Option<String>,
        path: Option<String>,
    },
    /// `unmap [PATH]`.
    Unmap { path: Option<String> },
    /// `unclaimed [--purge ID]`. An empty ID lists, as upstream's truthiness test does.
    Unclaimed { purge: Option<String> },
    /// `alias [NUM|EMAIL] [NAME] [--unset]`.
    Alias(AccountSetting),
    /// `group [NUM|EMAIL|ALIAS] [NAME] [--unset]`.
    Group(AccountSetting),
    /// `threshold [NUM|EMAIL] [PCT] [--unset]`. The value is validated in core.
    Threshold(AccountSetting),
    /// `priority [NUM|EMAIL|ALIAS] [TIER] [--unset]`. The value is validated in core.
    Priority(AccountSetting),
    /// `swap A B`.
    Swap { first: String, second: String },
    /// `move ACCOUNT SLOT`.
    Move { account: String, slot: String },
    /// `login [NUM|EMAIL|ALIAS] [--slot N] [--alias NAME] [--sso]`.
    Login(LoginOptions),
    /// `profile [list|show|save|export|import|apply|diff|delete|rollback]`.
    Profile(ProfileAction),
    /// `wake [status|on|off|config|run|install|uninstall]`.
    Wake(WakeAction),
    /// `desktop [status|path]`.
    Desktop(DesktopAction),
    /// `sessions [--json]`.
    Sessions { json: bool },
    /// `ai usage [index|report|quota|summary] [--json]`.
    Usage { action: UsageAction, json: bool },
    /// `login`, `logout`, `whoami`, `account delete` (top level): the HaryX account, not a Claude
    /// account.
    Account(AccountAction),
    /// `sync [...]` (top level): encrypted sync between the account's machines.
    Sync(SyncAction),
    /// `ai sessions continue SESSION [--to NUM|EMAIL|best] [--no-launch] [--json]`.
    SessionsContinue {
        session: String,
        to: String,
        json: bool,
        no_launch: bool,
    },
}

/// Options of `login`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoginOptions {
    /// A managed account to sign in again; `None` adds a new account.
    pub account: Option<String>,
    pub slot: Option<i64>,
    pub alias: Option<String>,
    pub sso: bool,
}

/// `switch --strategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SwitchStrategy {
    Best,
    NextAvailable,
}

/// `auto --strategy` (`settings.AUTOSWITCH_STRATEGIES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum AutoStrategy {
    Best,
    WeeklyFirst,
    ConsumeFirst,
    PriorityConsumeFirst,
}

/// Where `import` reads accounts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSource {
    /// An export file (`-` for stdin), as upstream.
    File(String),
    /// haryx only: the current cswap store on this machine.
    Cswap,
}

/// Options of `run`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOptions {
    /// `None`: use the current directory's mapping.
    pub account: Option<String>,
    pub no_share: bool,
    /// `--share-history` / `--no-share-history`; `None` leaves it to `run.shareHistory`.
    pub share_history: Option<bool>,
    pub require_session: bool,
    /// Everything after the first `--`, forwarded to claude verbatim.
    pub claude_args: Vec<String>,
}

/// Options of `auto`. `None` means "not given": settings.json decides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AutoOptions {
    pub once: bool,
    pub json: bool,
    pub interval: Option<f64>,
    pub threshold: Option<f64>,
    pub weekly_threshold: Option<f64>,
    pub cooldown: Option<f64>,
    pub model: Option<String>,
    pub include_api_key_accounts: Option<bool>,
    pub strategy: Option<AutoStrategy>,
    pub dry_run: bool,
}

/// `config` verbs. No verb is `List`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigAction {
    List,
    Get { key: String },
    Set { key: String, value: String },
    Unset { key: String },
    Path,
}

/// The shape shared by `alias`, `group`, `threshold` and `priority`: list all, set one, or clear one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountSetting {
    List,
    Set { account: String, value: String },
    Unset { account: String },
}

impl Command {
    /// The subcommand name, for messages.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Watch => "watch",
            Self::List { .. } => "list",
            Self::Status { .. } => "status",
            Self::Doctor { .. } => "doctor",
            Self::Switch { .. } | Self::SwitchTo { .. } => "switch",
            Self::Add { .. } => "add",
            Self::AddToken { .. } => "add-token",
            Self::Remove { .. } => "remove",
            Self::Disable { .. } => "disable",
            Self::Enable { .. } => "enable",
            Self::Export { .. } => "export",
            Self::Import { .. } => "import",
            Self::ImportUsage { .. } => "import-usage",
            Self::Purge => "purge",
            Self::Upgrade { .. } => "upgrade",
            Self::Run(_) => "run",
            Self::Auto(_) => "auto",
            Self::Config { .. } => "config",
            Self::Map { .. } => "map",
            Self::Unmap { .. } => "unmap",
            Self::Unclaimed { .. } => "unclaimed",
            Self::Alias(_) => "alias",
            Self::Group(_) => "group",
            Self::Threshold(_) => "threshold",
            Self::Priority(_) => "priority",
            Self::Swap { .. } => "swap",
            Self::Move { .. } => "move",
            Self::Login(_) => "login",
            Self::Profile(_) => "profile",
            Self::Wake(_) => "wake",
            Self::Desktop(_) => "desktop",
            Self::Sessions { .. } => "sessions",
            Self::SessionsContinue { .. } => "sessions continue",
            Self::Usage { .. } => "usage",
            Self::Account(AccountAction::Login { .. }) => "login",
            Self::Account(AccountAction::Logout { .. }) => "logout",
            Self::Account(AccountAction::Whoami { .. }) => "whoami",
            Self::Account(AccountAction::Delete { .. }) => "account delete",
            Self::Sync(_) => "sync",
        }
    }
}

/// Memorable subcommands and the long-standing flags they expand to (`_SUBCOMMAND_FLAGS`).
/// `switch` is special-cased in [`translate_subcommand`]; the pre-dispatched commands are not here.
pub const SUBCOMMAND_FLAGS: &[(&str, &str)] = &[
    ("help", "--help"),
    ("list", "--list"),
    ("ls", "--list"),
    ("status", "--status"),
    ("add", "--add-account"),
    ("add-token", "--add-token"),
    ("remove", "--remove-account"),
    ("rm", "--remove-account"),
    ("disable", "--disable-account"),
    ("enable", "--enable-account"),
    ("export", "--export"),
    ("import", "--import"),
    ("import-usage", "--import-usage"),
    ("purge", "--purge"),
    ("upgrade", "--upgrade"),
    ("update", "--upgrade"),
    ("tui", "--tui"),
    ("watch", "--watch"),
];

/// Rewrite a leading memorable subcommand into the equivalent flag argv (`_translate_subcommand`).
///
/// Only a recognized first token is rewritten; everything after it passes through verbatim, so
/// `haryx list --json` becomes `haryx --list --json`. A bare `switch` rotates (`--switch`);
/// `switch <num|email>` jumps to that account (`--switch-to <num|email>`).
pub fn translate_subcommand(argv: &[String]) -> Vec<String> {
    let Some((verb, rest)) = argv.split_first() else {
        return Vec::new();
    };
    let expanded = |flag: &str| {
        std::iter::once(flag.to_owned())
            .chain(rest.iter().cloned())
            .collect()
    };
    if verb == "switch" {
        let targeted = rest.first().is_some_and(|first| !first.starts_with('-'));
        return expanded(if targeted { "--switch-to" } else { "--switch" });
    }
    match SUBCOMMAND_FLAGS.iter().find(|(name, _)| name == verb) {
        Some((_, flag)) => expanded(flag),
        None => argv.to_vec(),
    }
}

/// The section every account, usage and session command lives under: `haryx ai list`.
pub const AI_SECTION: &str = "ai";

/// Commands that work only inside the `ai` section: their top-level names are kept for HaryX itself.
/// `login` left this list when `haryx login` became the HaryX account sign-in.
pub const AI_ONLY: &[&str] = &["profile", "wake", "desktop", "sessions", "usage", "doctor"];

/// Top-level help requests, answered with the section overview instead of the `ai` command list.
const TOP_HELP: &[&str] = &["help", "--help", "-h"];

/// Parse the arguments after the program name, as upstream `main()` dispatches them.
///
/// `haryx ai <command>` is the documented form. The commands that predate the sections keep working
/// without `ai` (`haryx list`), as upstream spells them; the [`AI_ONLY`] ones do not.
///
/// `interactive` says whether stdin and stdout are both terminals: bare `haryx` then opens the TUI,
/// and otherwise is the "no command given" usage error. Help and version requests come back as a
/// [`clap::Error`] of kind `DisplayHelp` / `DisplayVersion`; [`clap::Error::exit`] prints them on
/// stdout with exit code 0 and usage errors on stderr with exit code 2.
pub fn parse(argv: &[String], interactive: bool) -> Result<Invocation, Error> {
    match argv.split_first() {
        None if !interactive => Err(top::no_command()),
        Some((first, rest)) if first == AI_SECTION => parse_ai(rest, interactive),
        Some((first, rest)) if account_parser::ACCOUNT_VERBS.contains(&first.as_str()) => {
            account_parser::parse(first, rest)
        }
        Some((first, rest)) if first == "sync" => sync_parser::parse(rest),
        Some((first, _)) if AI_ONLY.contains(&first.as_str()) => Err(top::moved_to_ai(first)),
        Some((first, rest)) if rest.is_empty() && TOP_HELP.contains(&first.as_str()) => {
            Err(top::help())
        }
        _ => parse_ai(argv, interactive),
    }
}

/// Parse the arguments of the `ai` section (after `haryx ai`, or after a bare `haryx` for the
/// commands that predate the sections).
fn parse_ai(argv: &[String], interactive: bool) -> Result<Invocation, Error> {
    if let Some((first, rest)) = argv.split_first()
        && first == "profile"
    {
        return profile_parser::parse(rest);
    }
    if let Some((first, rest)) = argv.split_first()
        && first == "wake"
    {
        return wake_parser::parse(rest);
    }
    if let Some((first, rest)) = argv.split_first()
        && first == "desktop"
    {
        return desktop_parser::parse(rest);
    }
    if let Some((first, rest)) = argv.split_first()
        && first == "sessions"
    {
        return sessions_parser::parse(rest);
    }
    if let Some((first, rest)) = argv.split_first()
        && first == "usage"
    {
        return usage_parser::parse(rest);
    }
    if let Some((first, rest)) = argv.split_first()
        && let Some(parsed) = subparsers::parse(first, rest)
    {
        return parsed;
    }
    if argv.is_empty() && interactive {
        return Ok(Invocation {
            debug: false,
            command: Command::Tui,
        });
    }
    main_parser::parse(&translate_subcommand(argv))
}
