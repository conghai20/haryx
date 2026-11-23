//! `haryx ai wake`: the command side of [`haryx_core::wake`].

use std::collections::BTreeMap;
use std::time::Duration;

use haryx_core::Error;
use haryx_core::printer::dimmed;
use haryx_core::switcher::Switcher;
use haryx_core::wake::{
    self, AccountState, Outcome, QuietHours, WakeConfig, WakeStore, agent, parse_weekly_cap,
};
use jiff::Timestamp;
use serde_json::{Value, json};

use super::accounts::fail;
use super::profile_view::print_json;
use super::{CommandError, guard};
use crate::cli::WakeAction;

const EVENTS_SHOWN: usize = 10;

pub fn run(action: &WakeAction, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let store = WakeStore::new(switcher.backup_dir());
    match action {
        WakeAction::Status { json } => status(&switcher, &store, *json),
        WakeAction::On { accounts } => set(&switcher, &store, accounts, true),
        WakeAction::Off { accounts } => set(&switcher, &store, accounts, false),
        WakeAction::Config {
            quiet,
            no_quiet,
            weekly_cap,
            json,
        } => configure(&store, quiet.as_deref(), *no_quiet, *weekly_cap, *json),
        WakeAction::Run {
            once,
            dry_run,
            json,
        } => run_ticks(&switcher, &store, *once, *dry_run, *json),
        WakeAction::Install => install(&switcher, &store),
        WakeAction::Uninstall => uninstall(&switcher),
    }
}

fn iso(epoch: Option<f64>) -> Value {
    epoch
        .and_then(|e| Timestamp::from_millisecond((e * 1000.0) as i64).ok())
        .map_or(Value::Null, |t| json!(t.to_string()))
}

fn clock(epoch: Option<f64>) -> String {
    epoch
        .and_then(|e| Timestamp::from_millisecond((e * 1000.0) as i64).ok())
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}

/// Resolve the typed accounts to numbers; `all` means every managed account.
fn resolve(switcher: &Switcher, accounts: &[String]) -> Result<Vec<String>, CommandError> {
    if accounts
        .first()
        .is_some_and(|a| a.eq_ignore_ascii_case(wake::ALL))
    {
        return Ok(switcher
            .build_accounts_info()
            .iter()
            .map(|i| i.num())
            .collect());
    }
    accounts
        .iter()
        .map(|a| switcher.resolve_account(a).map(|(num, _, _)| num))
        .collect::<Result<_, Error>>()
        .map_err(fail)
}

fn set(
    switcher: &Switcher,
    store: &WakeStore,
    accounts: &[String],
    on: bool,
) -> Result<(), CommandError> {
    let (config, _) = store.load().map_err(fail)?;
    let numbers = resolve(switcher, accounts)?;
    let all = accounts
        .first()
        .is_some_and(|a| a.eq_ignore_ascii_case(wake::ALL));
    let next: Vec<String> = if on {
        config.accounts.iter().chain(&numbers).cloned().collect()
    } else if all {
        Vec::new()
    } else {
        config
            .accounts
            .iter()
            .filter(|n| !numbers.contains(n))
            .cloned()
            .collect()
    };
    let mut sorted = next;
    sorted.sort_by_key(|n| n.parse::<i64>().unwrap_or(i64::MAX));
    sorted.dedup();
    let updated = WakeConfig {
        accounts: sorted,
        ..config
    };
    store.save_config(&updated).map_err(fail)?;
    let changed = if numbers.is_empty() {
        "none".to_string()
    } else {
        numbers.join(", ")
    };
    println!("Wake {}: {changed}", if on { "on" } else { "off" });
    if on && !agent::is_installed(&switcher.path_env().home) {
        println!(
            "{}",
            dimmed("Nothing checks yet: run `haryx ai wake install` to check every minute.")
        );
    }
    Ok(())
}

fn configure(
    store: &WakeStore,
    quiet: Option<&str>,
    no_quiet: bool,
    weekly_cap: Option<f64>,
    json: bool,
) -> Result<(), CommandError> {
    let (config, _) = store.load().map_err(fail)?;
    let quiet_hours = match (quiet, no_quiet) {
        (Some(q), _) => Some(QuietHours::parse(q).map_err(fail)?),
        (None, true) => None,
        (None, false) => config.quiet_hours,
    };
    let weekly_cap_pct = match weekly_cap {
        Some(v) => parse_weekly_cap(v).map_err(fail)?,
        None => config.weekly_cap_pct,
    };
    let updated = WakeConfig {
        quiet_hours,
        weekly_cap_pct,
        ..config.clone()
    };
    if updated != config {
        store.save_config(&updated).map_err(fail)?;
    }
    if json {
        print_json(&config_json(&updated));
    } else {
        println!(
            "Quiet hours: {}",
            updated
                .quiet_hours
                .map_or("none".to_string(), QuietHours::format)
        );
        println!("Weekly cap: {}%", updated.weekly_cap_pct);
    }
    Ok(())
}

fn config_json(config: &WakeConfig) -> Value {
    json!({
        "accounts": config.accounts,
        "quietHours": config.quiet_hours.map(QuietHours::format),
        "weeklyCapPct": config.weekly_cap_pct,
    })
}

fn account_json(state: Option<&AccountState>) -> Value {
    let s = state.cloned().unwrap_or_default();
    json!({
        "lastWakeAt": iso(s.last_wake_at),
        "lastCheckAt": iso(s.last_check_at),
        "nextDue": iso(s.next_due),
        "lastResult": s.last_result,
    })
}

fn status(switcher: &Switcher, store: &WakeStore, json: bool) -> Result<(), CommandError> {
    let (config, state) = store.load().map_err(fail)?;
    let installed = agent::is_installed(&switcher.path_env().home);
    let infos = switcher.build_accounts_info();
    if json {
        let accounts: Vec<Value> = infos
            .iter()
            .map(|i| {
                let num = i.num();
                let mut row = account_json(state.get(&num));
                row["number"] = json!(i.number);
                row["email"] = json!(i.email);
                row["alias"] = json!(i.alias);
                row["on"] = json!(config.accounts.contains(&num));
                row
            })
            .collect();
        let mut doc = config_json(&config);
        doc["agentInstalled"] = json!(installed);
        doc["accounts"] = json!(accounts);
        doc["events"] = json!(store.recent_events(EVENTS_SHOWN));
        print_json(&doc);
        return Ok(());
    }
    println!(
        "Agent: {}   Quiet hours: {}   Weekly cap: {}%",
        if installed {
            "installed"
        } else {
            "not installed"
        },
        config
            .quiet_hours
            .map_or("none".to_string(), QuietHours::format),
        config.weekly_cap_pct
    );
    for info in &infos {
        let num = info.num();
        let on = config.accounts.contains(&num);
        let name = if info.alias.is_empty() {
            info.email.clone()
        } else {
            format!("{} ({})", info.alias, info.email)
        };
        let s = state.get(&num).cloned().unwrap_or_default();
        let detail = if on {
            format!(
                "next {}  last wake {}  {}",
                clock(s.next_due),
                clock(s.last_wake_at),
                s.last_result.unwrap_or_default()
            )
        } else {
            "off".to_string()
        };
        println!("  {num:>2}  {name:<40} {detail}");
    }
    Ok(())
}

fn print_outcomes(outcomes: &[Outcome], json: bool) {
    if json {
        let rows: Vec<Value> = outcomes
            .iter()
            .map(|o| json!({"number": o.number, "result": o.result, "nextDue": iso(o.next_due)}))
            .collect();
        print_json(&json!({ "accounts": rows }));
        return;
    }
    for o in outcomes {
        println!(
            "  {:>2}  {}  {}",
            o.number,
            o.result,
            dimmed(&clock(o.next_due))
        );
    }
}

/// A macOS notification for a failure the user should act on. Best effort.
fn notify(outcome: &Outcome) {
    if !cfg!(target_os = "macos") || !outcome.notify {
        return;
    }
    let text = if outcome.result == "skip:login" {
        format!(
            "Account {} needs signing in again: run haryx ai login {}",
            outcome.number, outcome.number
        )
    } else {
        format!(
            "Wake for account {} failed ({})",
            outcome.number, outcome.result
        )
    };
    let script = format!(
        "display notification {:?} with title \"HaryX\"",
        text.replace('"', "'")
    );
    if let Err(e) = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
    {
        log::warn!("wake: cannot send notification: {e}");
    }
}

fn run_ticks(
    switcher: &Switcher,
    store: &WakeStore,
    once: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), CommandError> {
    loop {
        let outcomes = wake::run_tick(
            switcher,
            store,
            wake::tick::now(),
            dry_run,
            &wake::send_probe,
        )
        .map_err(fail)?;
        if !dry_run {
            outcomes.iter().for_each(notify);
        }
        if once || dry_run {
            print_outcomes(&outcomes, json);
            return Ok(());
        }
        let woke: BTreeMap<_, _> = outcomes
            .iter()
            .filter(|o| o.result != "not-due")
            .map(|o| (o.number.clone(), o.result.clone()))
            .collect();
        if !woke.is_empty() {
            print_outcomes(&outcomes, false);
        }
        std::thread::sleep(Duration::from_secs(u64::from(agent::INTERVAL_S)));
    }
}

fn install(switcher: &Switcher, store: &WakeStore) -> Result<(), CommandError> {
    let exe = std::env::current_exe().map_err(|e| fail(Error::Io(e)))?;
    let log = store.dir().join("wake.log");
    let path = agent::install(&switcher.path_env().home, &exe, &log).map_err(fail)?;
    println!("Installed {}", path.display());
    println!(
        "{}",
        dimmed(&format!(
            "Checks every {}s with {}.",
            agent::INTERVAL_S,
            exe.display()
        ))
    );
    Ok(())
}

fn uninstall(switcher: &Switcher) -> Result<(), CommandError> {
    let removed = agent::uninstall(&switcher.path_env().home).map_err(fail)?;
    println!(
        "{}",
        if removed {
            "Wake agent removed."
        } else {
            "Wake agent was not installed."
        }
    );
    Ok(())
}
