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
//! `priority`, `swap`, `move`. As upstream, they must be the first argument (`haryx --debug run 2` is a
//! usage error; `haryx run 2 --debug` works).
//!
//! Dropped from upstream: `menubar` and its flags (`--install-service`, `--uninstall-service`,
//! `--service-status`, `--foreground`, `--auto-switch`, `--no-auto-switch`). All of them drive the
//! Python menu bar's LaunchAgent; the HaryX macOS app replaces that menu bar.
//!
//! Added: `import --from-cswap`, which copies accounts from an existing cswap store.

pub mod help;
mod main_parser;
mod subparsers;

pub use clap::Error;

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
    /// `upgrade` / `update` / `--upgrade`.
    Upgrade,
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
    /// `threshold [NUM|EMAIL] [PCT] [--unset]`. The value is validated in core.
    Threshold(AccountSetting),
    /// `priority [NUM|EMAIL|ALIAS] [TIER] [--unset]`. The value is validated in core.
    Priority(AccountSetting),
    /// `swap A B`.
    Swap { first: String, second: String },
    /// `move ACCOUNT SLOT`.
    Move { account: String, slot: String },
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

/// The shape shared by `alias`, `threshold` and `priority`: list all, set one, or clear one.
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
            Self::Upgrade => "upgrade",
            Self::Run(_) => "run",
            Self::Auto(_) => "auto",
            Self::Config { .. } => "config",
            Self::Map { .. } => "map",
            Self::Unmap { .. } => "unmap",
            Self::Unclaimed { .. } => "unclaimed",
            Self::Alias(_) => "alias",
            Self::Threshold(_) => "threshold",
            Self::Priority(_) => "priority",
            Self::Swap { .. } => "swap",
            Self::Move { .. } => "move",
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

/// Parse the arguments after the program name, as upstream `main()` dispatches them.
///
/// `interactive` says whether stdin and stdout are both terminals: bare `haryx` then opens the TUI,
/// and otherwise is the "no command given" usage error. Help and version requests come back as a
/// [`clap::Error`] of kind `DisplayHelp` / `DisplayVersion`; [`clap::Error::exit`] prints them on
/// stdout with exit code 0 and usage errors on stderr with exit code 2.
pub fn parse(argv: &[String], interactive: bool) -> Result<Invocation, Error> {
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
