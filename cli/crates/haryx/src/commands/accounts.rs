//! Account-management commands: `alias`, `threshold`, `priority`, `swap`, `move`, `map`, `unmap`,
//! `unclaimed`, `disable`, `enable`. Ports of the matching `cli.py` handlers; all logic is in
//! [`haryx_core::switcher::Switcher`].

use std::path::{Path, PathBuf};

use haryx_core::mappings::{MappingStore, normalize_path};
use haryx_core::models::pyfmt::format_g;
use haryx_core::models::{normalize_account_priority_str, normalize_account_threshold_str};
use haryx_core::printer::{accent, bolded, dimmed, muted, warning};
use haryx_core::settings::load_settings;
use haryx_core::switcher::Switcher;
use haryx_core::{Error, PROG};
use serde_json::Value;

use super::{CommandError, guard};
use crate::cli::{AccountSetting, ImportSource, RunOptions};

/// Open the store after the root guard, as every upstream handler does.
fn open(debug: bool) -> Result<Switcher, CommandError> {
    guard::refuse_root()?;
    Switcher::from_process(debug).map_err(fail)
}

/// Upstream's `except ClaudeSwitchError as e: error(f"Error: {e}"); sys.exit(1)`.
pub(super) fn fail(err: Error) -> CommandError {
    CommandError {
        message: format!("Error: {err}"),
        exit_code: 1,
    }
}

fn current_dir() -> Result<PathBuf, CommandError> {
    std::env::current_dir().map_err(|e| fail(Error::Io(e)))
}

/// Email of a slot in `sequence.json`, or "" (upstream `accounts.get(num, {}).get("email", "")`).
fn slot_email(switcher: &Switcher, num: &str) -> String {
    switcher
        .get_sequence_data()
        .ok()
        .flatten()
        .and_then(|data| {
            data.get("accounts")?
                .get(num)?
                .get("email")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn numeric_order(a: &str, b: &str) -> [String; 2] {
    let key = |s: &str| s.parse::<i64>().unwrap_or(0);
    if key(a) <= key(b) {
        [a.to_owned(), b.to_owned()]
    } else {
        [b.to_owned(), a.to_owned()]
    }
}

fn print_pair(switcher: &Switcher, a: &str, b: &str) {
    println!("{} Account {a} and Account {b}:", accent("Swapped"));
    for num in numeric_order(a, b) {
        println!("  {num}: {}", slot_email(switcher, &num));
    }
}

pub fn alias(setting: &AccountSetting, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    match setting {
        AccountSetting::List => {
            let rows = switcher.list_aliases().map_err(fail)?;
            if rows.is_empty() {
                println!("{}", dimmed("No aliases set"));
                return Ok(());
            }
            println!("{}", bolded("Aliases:"));
            for (num, name, email) in rows {
                println!("  {num}: {name} {}", muted(&format!("({email})")));
            }
        }
        AccountSetting::Unset { account } => {
            let num = switcher.unset_alias(account).map_err(fail)?;
            println!("{} for Account {num}", accent("Removed alias"));
        }
        AccountSetting::Set { account, value } => {
            let (num, normalized) = switcher.set_alias(account, value).map_err(fail)?;
            println!("{} '{normalized}' for Account {num}", accent("Set alias"));
        }
    }
    Ok(())
}

pub fn threshold(setting: &AccountSetting, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    match setting {
        AccountSetting::List => print_threshold_table(&switcher),
        AccountSetting::Unset { account } => {
            switcher.set_account_threshold(account, None).map_err(fail)
        }
        AccountSetting::Set { account, value } => {
            // Validated before any file is touched, so a rejected value leaves sequence.json untouched.
            let pct = normalize_account_threshold_str(value).map_err(fail)?;
            switcher
                .set_account_threshold(account, Some(pct))
                .map_err(fail)
        }
    }
}

fn print_threshold_table(switcher: &Switcher) -> Result<(), CommandError> {
    let default = load_settings(switcher.backup_dir()).threshold;
    println!("{}", bolded("Switch-away thresholds:"));
    println!("  {}  {}%", dimmed("global default"), format_g(default));
    let mut rows: Vec<(String, f64)> = switcher
        .account_policies()
        .map_err(fail)?
        .into_iter()
        .filter_map(|(num, policy)| policy.threshold.map(|t| (num, t)))
        .collect();
    // Numeric slots first in number order; anything else after (upstream's sort key).
    rows.sort_by_key(|(num, _)| {
        let numeric = num.bytes().all(|b| b.is_ascii_digit()) && !num.is_empty();
        (
            !numeric,
            if numeric {
                num.parse::<i64>().unwrap_or(0)
            } else {
                0
            },
        )
    });
    if rows.is_empty() {
        println!("{}", dimmed("  No per-account overrides set"));
        return Ok(());
    }
    for (num, value) in rows {
        println!("  Account-{num}  {}%", format_g(value));
    }
    Ok(())
}

pub fn priority(setting: &AccountSetting, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    match setting {
        AccountSetting::List => {
            println!(
                "{}",
                bolded("Account priorities (higher first; default 0):")
            );
            for (num, policy) in switcher.account_policies().map_err(fail)? {
                let email = switcher.account_email(&num).map_err(fail)?;
                println!("  Account-{num}  {email}  {}", policy.priority);
            }
            println!(
                "{}",
                dimmed(&format!(
                    "Used only by: {PROG} auto --strategy priority-consume-first"
                ))
            );
            Ok(())
        }
        AccountSetting::Unset { account } => {
            switcher.set_account_priority(account, None).map_err(fail)
        }
        AccountSetting::Set { account, value } => {
            let tier = normalize_account_priority_str(value).map_err(fail)?;
            switcher
                .set_account_priority(account, Some(i64::from(tier)))
                .map_err(fail)
        }
    }
}

pub fn swap(first: &str, second: &str, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    let (a, b) = switcher.swap_accounts(first, second).map_err(fail)?;
    print_pair(&switcher, &a, &b);
    Ok(())
}

pub fn move_account(account: &str, slot: &str, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    let (src, target, swapped) = switcher.move_account(account, slot).map_err(fail)?;
    if src == target {
        println!(
            "{} slot {target}: {}",
            dimmed("Already in"),
            slot_email(&switcher, &target)
        );
    } else if swapped {
        print_pair(&switcher, &src, &target);
    } else {
        println!(
            "{} {} to slot {target}",
            accent("Moved"),
            slot_email(&switcher, &target)
        );
    }
    Ok(())
}

pub fn map(account: Option<&str>, path: Option<&str>, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    let Some(account) = account else {
        return switcher.list_mappings().map_err(fail);
    };
    let store = MappingStore::new(switcher.backup_dir());
    let (num, email, org_uuid) = switcher.resolve_account(account).map_err(fail)?;
    let target = match path {
        Some(p) => PathBuf::from(p),
        None => current_dir()?,
    };
    if !target.is_dir() {
        warning(&format!(
            "Warning: {} is not an existing directory (mapping it anyway)",
            target.display()
        ));
    }
    let previous = store.get(&target).map_err(fail)?;
    store.set(&target, &email, &org_uuid).map_err(fail)?;
    let shown = normalize_path(&target).map_err(fail)?;
    let prev_email = previous
        .as_ref()
        .and_then(|p| p.get("email"))
        .and_then(Value::as_str)
        .filter(|prev| *prev != email);
    match prev_email {
        Some(prev) => println!(
            "{} {shown} → Account-{num} ({email}) {}",
            accent("Mapped"),
            muted(&format!("(was {prev})"))
        ),
        None => println!("{} {shown} → Account-{num} ({email})", accent("Mapped")),
    }
    Ok(())
}

pub fn unmap(path: Option<&str>, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    let store = MappingStore::new(switcher.backup_dir());
    let target = match path {
        Some(p) => PathBuf::from(p),
        None => current_dir()?,
    };
    let shown = normalize_path(Path::new(&target)).map_err(fail)?;
    if store.remove(&target).map_err(fail)? {
        println!("{} {shown}", accent("Unmapped"));
    } else {
        println!("{}", dimmed(&format!("No mapping for {shown}")));
    }
    Ok(())
}

pub fn unclaimed(purge: Option<&str>, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    let entries = switcher.list_unclaimed_credentials();
    if let Some(id) = purge.filter(|id| !id.is_empty()) {
        if !entries.contains_key(id) {
            return Err(fail(Error::General(format!("no unclaimed entry {id}"))));
        }
        switcher.remove_unclaimed_credential(id).map_err(fail)?;
        println!("{} {id}", accent("Purged"));
        return Ok(());
    }
    if entries.is_empty() {
        println!("{}", dimmed("No unclaimed credential entries"));
        return Ok(());
    }
    let text = |meta: &Value, key: &str| {
        meta.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let mut ids: Vec<&String> = entries.keys().collect();
    ids.sort();
    for id in ids {
        let meta = &entries[id];
        let slot = text(meta, "configSlot").unwrap_or_else(|| "?".to_owned());
        let reason =
            text(meta, "reason").unwrap_or_else(|| "orphaned (no manifest row)".to_owned());
        println!("{id}  slot {slot}  {reason}");
    }
    Ok(())
}

pub fn set_disabled(account: &str, disabled: bool, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    switcher
        .set_account_disabled(account, disabled)
        .map_err(fail)
}

pub fn add(slot: Option<i64>, alias: Option<&str>, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    switcher.add_account(slot, alias, false).map_err(fail)
}

pub fn login(opts: &crate::cli::LoginOptions, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    let request = haryx_core::switcher::LoginRequest {
        target: opts.account.clone(),
        slot: opts.slot,
        alias: opts.alias.clone(),
        sso: opts.sso,
        assume_yes: false,
    };
    switcher.login_account(&request).map(|_| ()).map_err(fail)
}

pub fn add_token(
    token: Option<&str>,
    email: Option<&str>,
    slot: Option<i64>,
    debug: bool,
) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    switcher
        .add_account_from_token(token, email, slot, false)
        .map_err(fail)
}

pub fn remove(account: &str, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    switcher.remove_account(account, false).map_err(fail)
}

pub fn purge(debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    switcher.purge().map_err(fail)
}

pub fn export(
    path: &str,
    account: Option<&str>,
    full: bool,
    debug: bool,
) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    haryx_core::transfer::export_accounts(&switcher, path, account, full).map_err(fail)
}

pub fn import(source: &ImportSource, force: bool, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    match source {
        ImportSource::File(path) => haryx_core::transfer::import_accounts(&switcher, path, force),
        ImportSource::Cswap => haryx_core::transfer::import_from_cswap(&switcher, force),
    }
    .map_err(fail)
}

pub fn import_usage(path: &str, hold: Option<f64>, debug: bool) -> Result<(), CommandError> {
    let switcher = open(debug)?;
    haryx_core::transfer::import_usage(&switcher, path, hold).map_err(fail)
}

/// First run with an empty haryx store and an existing cswap store: offer the import at a terminal,
/// or print a one-line hint on stderr. Never touches stdout, so `--json` output stays pure.
pub fn offer_cswap_import(switcher: &Switcher, json: bool) {
    use std::io::IsTerminal;
    if switcher.sequence_file().exists() {
        return;
    }
    let Some(ns) = haryx_core::transfer::find_cswap_store(switcher) else {
        return;
    };
    let interactive = !json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        eprintln!(
            "{}",
            muted(&format!(
                "Found a cswap store at {}; copy its accounts with: {PROG} import --from-cswap",
                ns.store_root.display()
            ))
        );
        return;
    }
    eprint!(
        "Found a cswap store at {}. Copy its accounts into haryx (cswap is left untouched)? [y/N] ",
        ns.store_root.display()
    );
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_ok()
        && answer.trim().eq_ignore_ascii_case("y")
        && let Err(err) = haryx_core::transfer::import_from_cswap(switcher, false)
    {
        haryx_core::printer::error(&format!("Error: {err}"));
    }
}

/// `run [NUM|EMAIL] [--no-share] [--[no-]share-history] [--require-session] [-- claude args]`.
pub fn run(opts: &RunOptions, debug: bool) -> Result<(), CommandError> {
    haryx_core::session_manager::redirect_stdout_until_exec();
    let switcher = open(debug)?;
    let mut manager = haryx_core::session_manager::SessionManager::new(&switcher);
    let request = haryx_core::session_manager::RunRequest {
        share: !opts.no_share,
        share_history: opts.share_history,
        require_session: opts.require_session,
    };
    let result = match &opts.account {
        Some(account) => manager.run(account, &opts.claude_args, &request),
        None => {
            let cwd = current_dir()?;
            match switcher.slot_for_directory(&cwd).map_err(fail)? {
                (Some(slot), _) => manager.run(&slot, &opts.claude_args, &request),
                (None, Some(email)) => {
                    warning(&format!(
                        "Mapped account {email} no longer exists — launching the default account."
                    ));
                    manager.exec_default(&opts.claude_args)
                }
                (None, None) => {
                    println!(
                        "{}",
                        dimmed(&format!(
                            "No account mapped for {} — launching the default account.",
                            cwd.display()
                        ))
                    );
                    manager.exec_default(&opts.claude_args)
                }
            }
        }
    };
    result.map_err(fail)
}
