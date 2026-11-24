//! The weekly usage summary toast (`haryx ai usage summary`), when turned on in the Usage pane.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::settings;
use crate::state::AppState;

const WEEK_S: u64 = 7 * 86_400;
const LOOP_INTERVAL: Duration = Duration::from_secs(3600);

pub fn due(enabled: bool, last: Option<u64>, now: u64) -> bool {
    enabled && last.is_none_or(|t| now.saturating_sub(t) >= WEEK_S)
}

/// One line per account: "Account 2: 340.0M tokens (about $158 at API prices), ran out 3 times".
pub fn lines(summary: &Value, name: &dyn Fn(i64) -> String) -> Vec<String> {
    summary["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let who = a["account"]
                .as_i64()
                .map_or("Unknown account".to_owned(), name);
            let tokens = a["tokens"].as_u64().unwrap_or(0);
            let mut line = format!("{who}: {} tokens", short(tokens));
            if let Some(cost) = a["cost"].as_f64() {
                line.push_str(&format!(" (about ${cost:.0} at API prices)"));
            }
            match a["ranOut"].as_u64() {
                Some(1) => line.push_str(", ran out once"),
                Some(n) if n > 1 => line.push_str(&format!(", ran out {n} times")),
                _ => {}
            }
            line
        })
        .collect()
}

fn short(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

pub fn start_loop(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LOOP_INTERVAL);
            tick(&app);
        }
    });
}

fn tick(app: &AppHandle) {
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    let prefs = state.prefs();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if !due(prefs.weekly_summary, prefs.last_weekly_summary, now) {
        return;
    }
    let Ok(cli) = state.cli() else { return };
    let Ok(summary) = cli.json(&["ai", "usage", "summary", "--since", "7d", "--json"]) else {
        return;
    };
    if summary["history"]["needsChoice"].as_bool() != Some(false) {
        return;
    }
    let accounts = state
        .accounts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let name = |n: i64| {
        accounts
            .iter()
            .find(|a| a.number == n)
            .map_or(format!("Account {n}"), |a| {
                a.alias.clone().unwrap_or_else(|| a.email.clone())
            })
    };
    let body = lines(&summary, &name);
    if body.is_empty() {
        return;
    }
    let mut prefs = prefs;
    prefs.last_weekly_summary = Some(now);
    let _ = settings::save(&state.config_dir, &prefs);
    *state.prefs.lock().unwrap_or_else(|e| e.into_inner()) = prefs;
    crate::notify::show(app, "Your week in Claude Code", &body.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn weekly_and_only_when_on() {
        assert!(!due(false, None, 1_000_000));
        assert!(due(true, None, 1_000_000));
        assert!(!due(true, Some(1_000_000), 1_000_000 + WEEK_S - 1));
        assert!(due(true, Some(1_000_000), 1_000_000 + WEEK_S));
    }

    #[test]
    fn lines_name_accounts_tokens_cost_and_limits() {
        let summary = json!({"accounts": [
            {"account": 2, "tokens": 339_224_281u64, "cost": 157.64, "ranOut": 3},
            {"account": null, "tokens": 134_030, "cost": null, "ranOut": 0}
        ]});
        let lines = lines(&summary, &|n| format!("Account {n}"));
        assert_eq!(
            lines[0],
            "Account 2: 339.2M tokens (about $158 at API prices), ran out 3 times"
        );
        assert_eq!(lines[1], "Unknown account: 134.0k tokens");
    }
}
