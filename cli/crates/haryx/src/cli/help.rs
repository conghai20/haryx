//! Help, description and epilog texts, ported from the argparse parsers in `claude_swap/cli.py`.
//!
//! Texts are written with `{prog}` / `{ai}` placeholders that [`p`] fills with [`haryx_core::PROG`] /
//! [`haryx_core::AI_PROG`], so the names live in one place. Examples say `haryx ai` where upstream says
//! `cswap`: every account command lives under the `ai` section.

use haryx_core::models::{
    ACCOUNT_PRIORITY_MAX, ACCOUNT_THRESHOLD_MAX, ACCOUNT_THRESHOLD_MIN, pyfmt::format_g,
};
use haryx_core::settings::{SETTING_SPECS, format_setting_value};
use haryx_core::{AI_PROG, PROG};

/// The per-account threshold range as help shows it (`50-99.9`), read from `haryx_core::models`.
fn threshold_range() -> String {
    format!(
        "{}-{}",
        format_g(ACCOUNT_THRESHOLD_MIN),
        format_g(ACCOUNT_THRESHOLD_MAX)
    )
}

/// argparse prints the usage line first, then the description, the options and the epilog.
pub const HELP_TEMPLATE: &str =
    "{usage-heading} {usage}\n\n{about-with-newline}\n{all-args}{after-help}";

/// Fill the `{ai}` (`haryx ai`) and `{prog}` (`haryx`) placeholders.
pub fn p(text: &str) -> String {
    text.replace("{ai}", AI_PROG).replace("{prog}", PROG)
}

/// `haryx ai <sub>`: the program name a pre-dispatched parser shows in its usage line.
pub fn sub_bin(sub: &str) -> String {
    format!("{AI_PROG} {sub}")
}

/// `--version` text: `haryx <ver> (claude-swap <upstream ver>)` (clap prefixes the name).
pub fn version() -> String {
    format!(
        "{} (claude-swap {})",
        haryx_core::VERSION,
        haryx_core::UPSTREAM_VERSION
    )
}

/// `haryx --help`: the sections, not their commands.
pub fn top_usage() -> String {
    p("{prog} <section> <command> [args] [options]")
}

pub fn top_description() -> String {
    p("HaryX: a control center for your AI workflow

Sections:
  {ai} <command>     AI coding accounts: usage, switching, sign-in, sessions
                         ('{ai} help' lists its commands)

Commands:
  {prog} help             show this help
  {prog} login            sign this machine in to your HaryX account
  {prog} logout           sign this machine out of HaryX
  {prog} whoami           show the HaryX account this machine uses
  {prog} sync             sync settings and accounts between your machines
  {prog} account delete   delete your HaryX account and everything synced
  {prog} upgrade          self-upgrade to latest

Aliases: update=upgrade. Bare '{prog}' in a terminal opens the AI dashboard ('{ai} tui').")
}

/// `haryx ai --help`: every account command.
pub fn main_usage() -> String {
    p("{ai} <command> [args] [options]")
}

pub fn main_description() -> String {
    p(
        "AI coding accounts (Claude Code): usage, switching, sign-in and sessions

Commands:
  {ai} help                       show this help
  {ai} list                       list managed accounts
  {ai} status                     show current account
  {ai} doctor [--json]            check Claude Code is installed and signed in
  {ai} switch                     rotate to the next account
  {ai} switch <num|email>         switch to a specific account
  {ai} add                        add the current account
  {ai} login [num|email]          sign an account in again, or add a new one
  {ai} add-token [TOKEN|-]        register a setup-token or API key
  {ai} remove <num|email>         remove an account
  {ai} disable <num|email>        hold an account out of auto-rotation
  {ai} enable <num|email>         return a disabled account to rotation
  {ai} threshold <num|email> <pct>  set one account's switch-away point
  {ai} threshold <num|email> --unset  clear it (use the global default)
  {ai} threshold                  list the global default and overrides
  {ai} priority <num|email> <tier>  set priority (higher first; default 0)
  {ai} priority                   list account priorities
  {ai} group <num|email> <name>   add an account to a group (it can be in several)
  {ai} group <num|email> --remove <name>  take it out of that group
  {ai} group                      list groups
  {ai} run <num|email> [-- ...]   run as an account, this terminal only
  {ai} run                        run the current dir's mapped account
  {ai} map <num|email> [path]     map a directory to an account
  {ai} map                        list directory mappings
  {ai} unmap [path]               remove a directory mapping
  {ai} alias <num|email> <name>   set a short alias for an account
  {ai} alias <num|email> --unset  remove an account's alias
  {ai} alias                      list all aliases
  {ai} swap <a> <b>               exchange two accounts' slot numbers
  {ai} move <a> <slot>            assign an account to a slot (swaps if taken)
  {ai} auto                       auto-switch when nearing rate limits
  {ai} config [set KEY VALUE]     show or change settings (settings.json)
  {ai} unclaimed [--purge ID]     list or drop stashed credential entries
  {ai} export <path>              export accounts
  {ai} import <path>              import accounts
  {ai} import --from-cswap        copy accounts from an existing cswap store
  {ai} import-usage <path>        adopt usage another machine read (list --json)
  {ai} profile [show|export|import|apply]  move your Claude Code setup between machines
  {ai} wake [on|off|status|install]  start the next 5h window right after a reset
  {ai} desktop [status|path]      each account's Claude Desktop data folder
  {ai} sessions [--json]          running Claude Code sessions and their accounts
  {ai} sessions continue <pid>    end a session and resume it on another account
  {ai} usage [report|quota|summary]  usage history: tokens and API-price cost
  {ai} tui                        interactive dashboard (also: bare {prog})
  {ai} watch                      dashboard, opened on the live watch page
  {ai} purge                      remove all {prog} data

Aliases: ls=list  rm=remove",
    )
}

pub fn main_epilog() -> String {
    p("Flags combine with subcommands:
  {ai} switch --strategy best           # pick the account with most quota left
  {ai} switch --strategy next-available # rotate, skipping rate-limited accounts
  {ai} switch user@example.com
  {ai} list --token-status
  {ai} list --json
  {ai} import-usage usage.json --hold 600  # adopt another machine's list --json
  {ai} import --from-cswap               # copy accounts from cswap
  {ai} add --slot 3                      # add to a specific slot
  {ai} add-token sk-ant-oat01-... --email me@example.com
  {ai} run 2 -- --resume                 # forward args after '--' to claude
  {ai} auto --once                       # single auto-switch tick (cron-friendly)
  {ai} config set autoswitch.threshold 80

The original spellings keep working: without 'ai' ({prog} list, {prog} switch 2) and as flags
({prog} --switch, {prog} --list, ...). Only 'login', 'profile', 'wake', 'desktop', 'sessions' and 'usage' need 'ai': their top-level names are kept for HaryX itself.")
}

pub fn run_epilog() -> String {
    p("Examples:
  {ai} run 2
  {ai} run user@example.com
  {ai} run 2 --no-share
  {ai} run 2 --no-share-history
  {ai} run 2 --require-session
  {ai} run 2 -- --resume")
}

pub fn map_epilog() -> String {
    p("Examples:
  {ai} map 2 ~/work/client-app
  {ai} map user@example.com          # map the current directory
  {ai} map                           # list all mappings")
}

pub fn swap_epilog() -> String {
    p("Examples:
  {ai} swap 1 2
  {ai} swap dev user@example.com")
}

pub fn login_epilog() -> String {
    p("Examples:
  {ai} login 3                   fix \"re-login needed\" on account 3
  {ai} login                     add a new account (the active login stays)
  {ai} login --slot 5 --alias w  add a new account into slot 5 as 'w'
  {ai} login 3 --sso             sign in with single sign-on

Claude Code (`claude auth login`) does the sign-in in your browser; {prog} only stores the login.")
}

pub fn move_epilog() -> String {
    p("Examples:
  {ai} move user@example.com 1   move an account onto shortcut 1
  {ai} move dev 1                by alias
  {ai} move 2 1                  by number (swaps if slot 1 is taken)")
}

pub fn alias_epilog() -> String {
    p("Examples:
  {ai} alias 2 dev
  {ai} alias user@example.com dev
  {ai} alias 2 --unset
  {ai} alias                         # list all aliases")
}

pub fn group_epilog() -> String {
    p("Examples:
  {ai} group 2 \"Client A\"         # add Account 2 to 'Client A'
  {ai} group 2 Work                # ...and to 'Work': an account can be in several
  {ai} group dev work              # 'work' joins an existing 'Work'
  {ai} group 2 --remove \"Client A\" # leave one group, keep the others
  {ai} group 2 --unset             # leave every group
  {ai} group --rename Work Office  # rename a group on every account in it
  {ai} group --delete \"Client A\"  # every account leaves it; the accounts stay
  {ai} group                       # list groups and their accounts")
}

pub fn threshold_epilog() -> String {
    format!(
        "{}\n\nValid range: {} (percent used).",
        p("Examples:
  {ai} threshold 2 85
  {ai} threshold user@example.com 99.9
  {ai} threshold 2 --unset           # back to the global default
  {ai} threshold                     # list the global default and overrides"),
        threshold_range()
    )
}

pub fn threshold_pct_help() -> String {
    format!(
        "Percent used at which to switch away ({}).",
        threshold_range()
    )
}

pub fn priority_tier_help() -> String {
    format!("Integer from 0 to {ACCOUNT_PRIORITY_MAX} (default 0)")
}

pub fn auto_epilog() -> String {
    p("Exit codes with --once:
  0  switched to another account
  1  error (network trouble, lock contention, ...)
  2  no action needed
  3  blocked: wanted to switch but no viable target / all exhausted

Examples:
  {ai} auto                       # foreground loop, switch at 90% used
  {ai} auto --threshold 80        # switch earlier
  {ai} auto --weekly-threshold 97 # let the weekly window run higher before switching
  {ai} auto --model Fable         # also switch when the Fable weekly limit is hit
  {ai} auto --json                # one JSON event per line (for scripts)
  {ai} auto --once; echo $?       # single tick, outcome in exit code
  {ai} auto --dry-run             # log decisions, never actually switch

Defaults live in settings.json in the backup root; flags override them.")
}

/// Width of the key column in the `config` epilog (`{spec.dotted:<34}` plus the two-space indent).
const KEY_COLUMN: usize = 36;
/// The `config` epilog is pre-wrapped to this width, with a hanging indent under the help column,
/// so clap's own wrapping (which drops the indent) leaves it alone.
const EPILOG_WIDTH: usize = 80;

/// Wrap `text` into lines of at most `width` characters (a longer word gets a line of its own).
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    text.split_whitespace().fold(Vec::new(), |mut lines, word| {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + word.len() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
        lines
    })
}

fn key_line(key: &str, help: &str, default: &str) -> String {
    let text = format!("{help} (default {default})");
    let indent = " ".repeat(KEY_COLUMN);
    let lines = wrap_words(&text, EPILOG_WIDTH - KEY_COLUMN);
    let body = lines.join(&format!("\n{indent}"));
    format!("  {key:<34}{body}")
}

/// The `config` epilog: one line per `haryx_core::settings::SETTING_SPECS` key (dotted name, help,
/// default as `format_setting_value` renders it), then examples.
pub fn config_epilog() -> String {
    let keys: Vec<String> = SETTING_SPECS
        .iter()
        .map(|spec| {
            key_line(
                &spec.dotted(),
                &spec.help_text(),
                &format_setting_value(&spec.default()),
            )
        })
        .collect();
    format!(
        "Keys:\n{}\n\n{}",
        keys.join("\n"),
        p("Examples:
  {ai} config                              # list effective settings
  {ai} config get autoswitch.threshold
  {ai} config set autoswitch.threshold 80
  {ai} config set autoswitch.weeklyThreshold 97  # own line for the 7d/per-model windows
  {ai} config unset autoswitch.threshold   # back to the default
  {ai} config path                         # where settings.json lives")
    )
}
