//! `haryx ai usage`: the command side of [`haryx_core::usage_history`].

use std::io::IsTerminal;

use haryx_core::paths;
use haryx_core::printer::{dimmed, warning};
use haryx_core::settings::{load_history_settings, set_setting};
use haryx_core::switcher::Switcher;
use haryx_core::usage_history::{index, pricing, report};
use haryx_core::{AI_PROG, PROG};
use serde_json::{Value, json};

use super::accounts::fail;
use super::profile_view::print_json;
use super::{CommandError, guard};
use crate::cli::UsageAction;

pub fn run(action: &UsageAction, json: bool, debug: bool) -> Result<(), CommandError> {
    guard::refuse_root()?;
    let switcher = Switcher::from_process(debug).map_err(fail)?;
    let store = switcher.backup_dir().to_path_buf();
    let default_profile = paths::claude_config_home_in(switcher.path_env());
    let now = jiff::Timestamp::now().as_second();
    if let UsageAction::Index {
        import: Some(choice),
        ..
    } = action
    {
        set_setting(&store, "history.import", choice).map_err(fail)?;
    }
    let rebuild = matches!(action, UsageAction::Index { rebuild: true, .. });
    let mut outcome = index::run(&store, &default_profile, rebuild).map_err(user_error)?;
    if outcome.needs_choice && !json && std::io::stdin().is_terminal() && ask_import(&store)? {
        outcome = index::run(&store, &default_profile, false).map_err(user_error)?;
    }
    let status = json!({
        "enabled": !outcome.disabled,
        "needsChoice": outcome.needs_choice,
        "readReplies": outcome.replies,
        "keepDays": load_history_settings(&store).keep_days,
    });
    let since = |text: &str| {
        report::parse_since(text, now).ok_or_else(|| CommandError {
            message: format!(
                "Error: --since takes 24h, 7d, 30d, all or a date (YYYY-MM-DD), not {text:?}"
            ),
            exit_code: 2,
        })
    };
    let payload = match action {
        UsageAction::Index { .. } => status.clone(),
        UsageAction::Report {
            since: s,
            by,
            account,
            project,
        } => {
            let q = report::Query {
                since: since(s)?,
                by: report::By::parse(by).unwrap_or(report::By::Account),
                account: *account,
                project: project.clone(),
            };
            report::report(&report::rows(&store, q.since), &q)
        }
        UsageAction::Quota { since: s, account } => {
            report::quota_report(&store, since(s)?, *account)
        }
        UsageAction::Summary { since: s } => report::summary(&store, since(s)?),
    };
    if json {
        let mut payload = payload;
        if let Value::Object(map) = &mut payload {
            map.insert("history".into(), status);
        }
        print_json(&payload);
        return Ok(());
    }
    if outcome.disabled {
        println!("Usage history is off ({AI_PROG} config set history.enabled true).");
        return Ok(());
    }
    if outcome.needs_choice {
        println!(
            "Usage history has not started yet. Choose with {AI_PROG} usage index --import all \
             (read your past transcripts) or --import none (from now on)."
        );
        return Ok(());
    }
    match action {
        UsageAction::Index { .. } => println!("Read {} new replies.", outcome.replies),
        UsageAction::Report { .. } => print_report(&payload),
        UsageAction::Quota { .. } => print_quota(&payload),
        UsageAction::Summary { .. } => print_summary(&payload),
    }
    Ok(())
}

fn user_error(message: String) -> CommandError {
    CommandError {
        message: format!("Error: {message}"),
        exit_code: 1,
    }
}

/// The first-run question, in a terminal.
fn ask_import(store: &std::path::Path) -> Result<bool, CommandError> {
    println!("{PROG} can read your past Claude Code transcripts to build the usage history.");
    println!("It reads token counts only, and keeps them on this machine.");
    print!("Read past transcripts? [a]ll / [n]one, from now on: ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return Ok(false);
    }
    let choice = match answer.trim().to_ascii_lowercase().chars().next() {
        Some('a') => "all",
        Some('n') => "none",
        _ => return Ok(false),
    };
    set_setting(store, "history.import", choice).map_err(fail)?;
    Ok(true)
}

fn tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

fn cost(v: &Value) -> String {
    v.as_f64().map_or("—".into(), |c| format!("${c:.2}"))
}

fn print_report(r: &Value) {
    let rows = r["rows"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("No usage recorded in this range.");
        return;
    }
    let by = r["by"].as_str().unwrap_or("account");
    println!(
        "{:<44} {:>9} {:>10} {:>6}",
        by, "tokens", "API cost", "share"
    );
    for row in &rows {
        let key = row["key"].as_str().unwrap_or("");
        let key = if key.chars().count() > 43 {
            format!(
                "…{}",
                key.chars()
                    .rev()
                    .take(42)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>()
            )
        } else {
            key.to_owned()
        };
        let approx = if row["approximate"].as_bool() == Some(true) {
            " ~"
        } else {
            ""
        };
        println!(
            "{:<44} {:>9} {:>10} {:>5}%{approx}",
            key,
            tokens(row["tokens"].as_u64().unwrap_or(0)),
            cost(&row["cost"]),
            row["share"]
        );
    }
    let t = &r["total"];
    println!(
        "{:<44} {:>9} {:>10}",
        "total",
        tokens(t["tokens"].as_u64().unwrap_or(0)),
        cost(&t["cost"])
    );
    println!(
        "{}",
        dimmed(&format!(
            "Cost at API prices as of {} (plans are flat-priced). ~ = account inferred from the switch log.",
            pricing::as_of()
        ))
    );
}

fn print_quota(r: &Value) {
    let readings = r["readings"].as_array().cloned().unwrap_or_default();
    if readings.is_empty() {
        println!("No readings in this range yet; they are recorded as {PROG} reads usage.");
        return;
    }
    let tz = jiff::tz::TimeZone::system();
    for x in &readings {
        let at = jiff::Timestamp::from_second(x["at"].as_i64().unwrap_or(0))
            .unwrap_or_default()
            .to_zoned(tz.clone())
            .strftime("%b %d %H:%M");
        let pct = |v: &Value| v.as_f64().map_or("—".into(), |p| format!("{p:.0}%"));
        println!(
            "{at}  account {}  5h {:>5}  7d {:>5}",
            x["account"],
            pct(&x["fiveHour"]),
            pct(&x["sevenDay"])
        );
    }
}

fn print_summary(r: &Value) {
    let accounts = r["accounts"].as_array().cloned().unwrap_or_default();
    if accounts.is_empty() {
        println!("No usage recorded in this range.");
        return;
    }
    for a in &accounts {
        let name = a["account"]
            .as_i64()
            .map_or("Unknown account".into(), |n| format!("Account {n}"));
        let mut line = format!(
            "{name}: {} tokens ({}%), {} at API prices",
            tokens(a["tokens"].as_u64().unwrap_or(0)),
            a["share"],
            cost(&a["cost"])
        );
        match a["ranOut"].as_u64() {
            Some(1) => line.push_str(", ran out once"),
            Some(n) if n > 1 => line.push_str(&format!(", ran out {n} times")),
            _ => {}
        }
        if let (Some(p), Some(share)) = (a["topProject"].as_str(), a["topProjectShare"].as_f64()) {
            line.push_str(&format!("; {share:.0}% in {p}"));
        }
        println!("{line}");
    }
    if accounts.iter().any(|a| a["cost"].is_null()) {
        warning("Some models are not in the price table; their tokens show no cost.");
    }
}
