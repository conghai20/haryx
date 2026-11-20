//! Help, description and epilog texts, ported from the argparse parsers in `claude_swap/cli.py`.
//!
//! Texts are written with a `{prog}` placeholder that [`p`] fills with [`haryx_core::PROG`], so the
//! program name lives in one place. Examples say `haryx` where upstream says `cswap`.

use haryx_core::PROG;
use haryx_core::models::{
    ACCOUNT_PRIORITY_MAX, ACCOUNT_THRESHOLD_MAX, ACCOUNT_THRESHOLD_MIN, pyfmt::format_g,
};
use haryx_core::settings::{SETTING_SPECS, format_setting_value};

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

/// Fill the `{prog}` placeholder.
pub fn p(text: &str) -> String {
    text.replace("{prog}", PROG)
}

/// `haryx <sub>`: the program name a pre-dispatched parser shows in its usage line.
pub fn sub_bin(sub: &str) -> String {
    format!("{PROG} {sub}")
}

/// `--version` text: `haryx <ver> (claude-swap <upstream ver>)` (clap prefixes the name).
pub fn version() -> String {
    format!(
        "{} (claude-swap {})",
        haryx_core::VERSION,
        haryx_core::UPSTREAM_VERSION
    )
}

pub fn main_usage() -> String {
    p("{prog} <command> [args] [options]")
}

pub fn main_description() -> String {
    p("Multi-Account Switcher for Claude Code

Commands:
  {prog} help                       show this help
  {prog} list                       list managed accounts
  {prog} status                     show current account
  {prog} switch                     rotate to the next account
  {prog} switch <num|email>         switch to a specific account
  {prog} add                        add the current account
  {prog} add-token [TOKEN|-]        register a setup-token or API key
  {prog} remove <num|email>         remove an account
  {prog} disable <num|email>        hold an account out of auto-rotation
  {prog} enable <num|email>         return a disabled account to rotation
  {prog} threshold <num|email> <pct>  set one account's switch-away point
  {prog} threshold <num|email> --unset  clear it (use the global default)
  {prog} threshold                  list the global default and overrides
  {prog} priority <num|email> <tier>  set priority (higher first; default 0)
  {prog} priority                   list account priorities
  {prog} run <num|email> [-- ...]   run as an account, this terminal only
  {prog} run                        run the current dir's mapped account
  {prog} map <num|email> [path]     map a directory to an account
  {prog} map                        list directory mappings
  {prog} unmap [path]               remove a directory mapping
  {prog} alias <num|email> <name>   set a short alias for an account
  {prog} alias <num|email> --unset  remove an account's alias
  {prog} alias                      list all aliases
  {prog} swap <a> <b>               exchange two accounts' slot numbers
  {prog} move <a> <slot>            assign an account to a slot (swaps if taken)
  {prog} auto                       auto-switch when nearing rate limits
  {prog} config [set KEY VALUE]     show or change settings (settings.json)
  {prog} unclaimed [--purge ID]     list or drop stashed credential entries
  {prog} export <path>              export accounts
  {prog} import <path>              import accounts
  {prog} import --from-cswap        copy accounts from an existing cswap store
  {prog} import-usage <path>        adopt usage another machine read (list --json)
  {prog} tui                        interactive dashboard (also: bare {prog})
  {prog} watch                      dashboard, opened on the live watch page
  {prog} upgrade                    self-upgrade to latest
  {prog} purge                      remove all {prog} data

Aliases: ls=list  rm=remove  update=upgrade")
}

pub fn main_epilog() -> String {
    p("Flags combine with subcommands:
  {prog} switch --strategy best           # pick the account with most quota left
  {prog} switch --strategy next-available # rotate, skipping rate-limited accounts
  {prog} switch user@example.com
  {prog} list --token-status
  {prog} list --json
  {prog} import-usage usage.json --hold 600  # adopt another machine's list --json
  {prog} import --from-cswap               # copy accounts from cswap
  {prog} add --slot 3                      # add to a specific slot
  {prog} add-token sk-ant-oat01-... --email me@example.com
  {prog} run 2 -- --resume                 # forward args after '--' to claude
  {prog} auto --once                       # single auto-switch tick (cron-friendly)
  {prog} config set autoswitch.threshold 80

The original flag spellings ({prog} --switch, {prog} --list, ...) keep working.")
}

pub fn run_epilog() -> String {
    p("Examples:
  {prog} run 2
  {prog} run user@example.com
  {prog} run 2 --no-share
  {prog} run 2 --no-share-history
  {prog} run 2 --require-session
  {prog} run 2 -- --resume")
}

pub fn map_epilog() -> String {
    p("Examples:
  {prog} map 2 ~/work/client-app
  {prog} map user@example.com          # map the current directory
  {prog} map                           # list all mappings")
}

pub fn swap_epilog() -> String {
    p("Examples:
  {prog} swap 1 2
  {prog} swap dev user@example.com")
}

pub fn move_epilog() -> String {
    p("Examples:
  {prog} move user@example.com 1   move an account onto shortcut 1
  {prog} move dev 1                by alias
  {prog} move 2 1                  by number (swaps if slot 1 is taken)")
}

pub fn alias_epilog() -> String {
    p("Examples:
  {prog} alias 2 dev
  {prog} alias user@example.com dev
  {prog} alias 2 --unset
  {prog} alias                         # list all aliases")
}

pub fn threshold_epilog() -> String {
    format!(
        "{}\n\nValid range: {} (percent used).",
        p("Examples:
  {prog} threshold 2 85
  {prog} threshold user@example.com 99.9
  {prog} threshold 2 --unset           # back to the global default
  {prog} threshold                     # list the global default and overrides"),
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
  {prog} auto                       # foreground loop, switch at 90% used
  {prog} auto --threshold 80        # switch earlier
  {prog} auto --weekly-threshold 97 # let the weekly window run higher before switching
  {prog} auto --model Fable         # also switch when the Fable weekly limit is hit
  {prog} auto --json                # one JSON event per line (for scripts)
  {prog} auto --once; echo $?       # single tick, outcome in exit code
  {prog} auto --dry-run             # log decisions, never actually switch

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
  {prog} config                              # list effective settings
  {prog} config get autoswitch.threshold
  {prog} config set autoswitch.threshold 80
  {prog} config set autoswitch.weeklyThreshold 97  # own line for the 7d/per-model windows
  {prog} config unset autoswitch.threshold   # back to the default
  {prog} config path                         # where settings.json lives")
    )
}
