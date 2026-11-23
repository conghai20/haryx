//! One wake tick over the opted-in accounts.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use jiff::{Timestamp, tz::TimeZone};
use serde_json::{Map, Value, json};

use super::config::WakeConfig;
use super::decide::{Decision, Now, SkipReason, WindowRead, decide};
use super::state::{AccountState, WakeStore};
use crate::error::Result;
use crate::locking::FileLock;
use crate::oauth::{self, extract_oauth_data, is_oauth_token_expired};
use crate::switcher::Switcher;
use crate::usage_store::UsageEntry;

const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// What happened to one account in a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub number: String,
    /// `woke`, `would-wake`, `not-due`, `skip:<reason>` or `failed:<kind>`.
    pub result: String,
    pub next_due: Option<f64>,
    /// A failure the user should hear about (first time it happens).
    pub notify: bool,
}

/// Sends the wake request with an access token: the probe's reading, or the failure kind.
pub type SendFn<'a> = &'a dyn Fn(&str) -> std::result::Result<Map<String, Value>, String>;

/// The real sender: the 1-output-token header probe.
pub fn send_probe(access_token: &str) -> std::result::Result<Map<String, Value>, String> {
    oauth::Api::from_env()
        .request_usage_via_probe(access_token)
        .map_err(|e| e.classify().0)
}

pub fn now() -> Now {
    let ts = Timestamp::now();
    let zoned = ts.to_zoned(TimeZone::system());
    Now {
        epoch: ts.as_millisecond() as f64 / 1000.0,
        local_minute: (zoned.hour() as u16) * 60 + zoned.minute() as u16,
    }
}

/// An epoch from a `resets_at` value (ISO string or epoch number).
pub fn epoch_of(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s
            .parse::<Timestamp>()
            .ok()
            .map(|t| t.as_millisecond() as f64 / 1000.0),
        _ => None,
    }
}

/// The windows a usage reading reports (stored `{pct}` or raw `{utilization}` shapes).
pub fn window_read(usage: Option<&Value>) -> WindowRead {
    let Some(usage) = usage else {
        return WindowRead::default();
    };
    let pct = |w: &Value| {
        w.get("pct")
            .or_else(|| w.get("utilization"))
            .and_then(Value::as_f64)
    };
    WindowRead {
        five_hour_resets_at: usage
            .get("five_hour")
            .and_then(|w| w.get("resets_at"))
            .and_then(epoch_of),
        seven_day_pct: usage.get("seven_day").and_then(pct),
    }
}

fn is_failure(result: &str) -> bool {
    result.starts_with("failed:") || result == "skip:login"
}

/// Run one tick: decide from stored readings, refresh the due ones, and wake those still due.
pub fn run_tick(
    switcher: &Switcher,
    store: &WakeStore,
    now: Now,
    dry_run: bool,
    send: SendFn<'_>,
) -> Result<Vec<Outcome>> {
    let _lock = FileLock::lock(store.lock_path(), LOCK_TIMEOUT)?;
    let (config, mut state) = store.load()?;
    if config.accounts.is_empty() {
        return Ok(Vec::new());
    }
    let switchable: HashSet<String> = switcher.switchable_account_numbers()?.into_iter().collect();
    let stored = switcher.usage_entries_by_account(Some(&HashSet::new()), true);

    let mut results: BTreeMap<String, (String, Option<f64>)> = BTreeMap::new();
    let mut due = HashSet::new();
    for num in &config.accounts {
        if !switchable.contains(num) {
            results.insert(num.clone(), ("skip:disabled".into(), None));
            continue;
        }
        let entry_state = state.get(num).cloned().unwrap_or_default();
        let read = window_read(stored.get(num).and_then(|e| e.last_good.as_ref()));
        match decide(&read, now, &config, &entry_state) {
            Decision::NotDue { next } => {
                results.insert(num.clone(), ("not-due".into(), Some(next)));
            }
            Decision::Skip(reason) => {
                results.insert(num.clone(), (skip(reason), None));
            }
            Decision::Wake => {
                due.insert(num.clone());
            }
        }
    }

    if !due.is_empty() {
        wake_due(
            switcher,
            &config,
            &state,
            &due,
            now,
            dry_run,
            send,
            &mut results,
        );
    }
    let persist = if dry_run { None } else { Some(store) };
    Ok(record(persist, &mut state, results, now))
}

fn skip(reason: SkipReason) -> String {
    format!("skip:{}", reason.as_str())
}

/// Refresh the due accounts' readings (free usage endpoint) and wake the ones still due.
#[allow(clippy::too_many_arguments)]
fn wake_due(
    switcher: &Switcher,
    config: &WakeConfig,
    state: &BTreeMap<String, AccountState>,
    due: &HashSet<String>,
    now: Now,
    dry_run: bool,
    send: SendFn<'_>,
    results: &mut BTreeMap<String, (String, Option<f64>)>,
) {
    let fresh = switcher.usage_entries_by_account(Some(due), false);
    let infos = switcher.build_accounts_info();
    for num in due {
        let entry = fresh.get(num);
        if entry.is_some_and(UsageEntry::token_dead) {
            results.insert(num.clone(), (skip(SkipReason::Login), None));
            continue;
        }
        let read = window_read(entry.and_then(|e| e.last_good.as_ref()));
        let entry_state = state.get(num).cloned().unwrap_or_default();
        let outcome = match decide(&read, now, config, &entry_state) {
            Decision::NotDue { next } => ("not-due".to_string(), Some(next)),
            Decision::Skip(reason) => (skip(reason), None),
            Decision::Wake => {
                let creds = infos
                    .iter()
                    .find(|i| &i.num() == num)
                    .map(|i| i.credentials.as_str())
                    .unwrap_or_default();
                send_wake(creds, dry_run, send)
            }
        };
        results.insert(num.clone(), outcome);
    }
}

/// Send one wake with the account's credentials.
fn send_wake(credentials: &str, dry_run: bool, send: SendFn<'_>) -> (String, Option<f64>) {
    let oauth = extract_oauth_data(credentials);
    let token = oauth
        .as_ref()
        .and_then(|o| o.get("accessToken"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        return (skip(SkipReason::Login), None);
    };
    if oauth
        .as_ref()
        .is_some_and(|o| is_oauth_token_expired(o.get("expiresAt")))
    {
        return (skip(SkipReason::Login), None);
    }
    if dry_run {
        return ("would-wake".into(), None);
    }
    match send(token) {
        Ok(reading) if reading.contains_key("rejected") => ("failed:rejected".into(), None),
        Ok(reading) => {
            let next = window_read(Some(&Value::Object(reading)))
                .five_hour_resets_at
                .map(|r| r + super::decide::GRACE_S);
            ("woke".into(), next)
        }
        Err(kind) => (format!("failed:{kind}"), None),
    }
}

/// Fold the results into `state`; with a store, also log changed results and save (not on a dry run).
fn record(
    store: Option<&WakeStore>,
    state: &mut BTreeMap<String, AccountState>,
    results: BTreeMap<String, (String, Option<f64>)>,
    now: Now,
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for (num, (result, next_due)) in results {
        let previous = state.remove(&num).unwrap_or_default();
        let changed = previous.last_result.as_deref() != Some(result.as_str());
        let notify = is_failure(&result) && changed;
        let woke = result == "woke";
        if let Some(store) = store.filter(|_| changed || woke) {
            store.append_event(&json!({
                "at": now.epoch,
                "account": num,
                "result": result,
            }));
        }
        let next = AccountState {
            last_wake_at: if woke {
                Some(now.epoch)
            } else {
                previous.last_wake_at
            },
            last_check_at: Some(now.epoch),
            next_due: next_due.or(if result == "not-due" {
                previous.next_due
            } else {
                None
            }),
            last_result: Some(result.clone()),
            notified_at: if notify {
                Some(now.epoch)
            } else {
                previous.notified_at
            },
        };
        state.insert(num.clone(), next);
        outcomes.push(Outcome {
            number: num,
            result,
            next_due,
            notify,
        });
    }
    if let Some(Err(e)) = store.map(|s| s.save_state(state)) {
        log::warn!("wake: cannot save state: {e}");
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_shape() {
        let usage = json!({
            "five_hour": {"pct": 12.0, "resets_at": "2026-11-24T10:00:00Z"},
            "seven_day": {"pct": 55.0}
        });
        let read = window_read(Some(&usage));
        assert_eq!(read.seven_day_pct, Some(55.0));
        assert_eq!(read.five_hour_resets_at, Some(1_795_514_400.0));
    }

    #[test]
    fn reads_probe_shape_and_epoch_numbers() {
        let usage = json!({
            "five_hour": {"utilization": 0.0, "resets_at": 1_800_000_000.0},
            "seven_day": {"utilization": 91.0}
        });
        let read = window_read(Some(&usage));
        assert_eq!(read.five_hour_resets_at, Some(1_800_000_000.0));
        assert_eq!(read.seven_day_pct, Some(91.0));
    }

    #[test]
    fn missing_reading_is_empty() {
        assert_eq!(window_read(None), WindowRead::default());
        assert_eq!(
            window_read(Some(&json!({"five_hour": null}))),
            WindowRead::default()
        );
    }

    fn creds(expires_ms: i64) -> String {
        json!({"claudeAiOauth": {"accessToken": "tok", "expiresAt": expires_ms}}).to_string()
    }

    #[test]
    fn send_wake_reports_woke_with_next_due() {
        let send = |t: &str| {
            assert_eq!(t, "tok");
            let mut m = Map::new();
            m.insert(
                "five_hour".into(),
                json!({"utilization": 0.0, "resets_at": 100.0}),
            );
            Ok(m)
        };
        let (result, next) = send_wake(&creds(i64::MAX / 2), false, &send);
        assert_eq!(result, "woke");
        assert_eq!(next, Some(105.0));
    }

    #[test]
    fn send_wake_dry_run_sends_nothing() {
        let send = |_: &str| -> std::result::Result<Map<String, Value>, String> {
            panic!("must not send")
        };
        assert_eq!(send_wake(&creds(i64::MAX / 2), true, &send).0, "would-wake");
    }

    #[test]
    fn send_wake_expired_or_missing_token_is_login() {
        let send = |_: &str| -> std::result::Result<Map<String, Value>, String> {
            panic!("must not send")
        };
        assert_eq!(send_wake(&creds(1), false, &send).0, "skip:login");
        assert_eq!(send_wake("", false, &send).0, "skip:login");
    }

    #[test]
    fn send_wake_maps_failures() {
        let rejected = |_: &str| {
            let mut m = Map::new();
            m.insert("rejected".into(), json!({}));
            Ok(m)
        };
        assert_eq!(
            send_wake(&creds(i64::MAX / 2), false, &rejected).0,
            "failed:rejected"
        );
        let http = |_: &str| Err("http-401".to_string());
        assert_eq!(
            send_wake(&creds(i64::MAX / 2), false, &http).0,
            "failed:http-401"
        );
    }

    #[test]
    fn record_notifies_once_per_failure_and_stamps_wakes() {
        let dir = tempfile::tempdir().unwrap();
        let store = WakeStore::new(dir.path());
        let mut state = BTreeMap::new();
        let at = Now {
            epoch: 50.0,
            local_minute: 0,
        };
        let fail = BTreeMap::from([("1".to_string(), ("failed:http-401".to_string(), None))]);
        assert!(record(Some(&store), &mut state, fail.clone(), at)[0].notify);
        assert!(!record(Some(&store), &mut state, fail, at)[0].notify);
        let woke = BTreeMap::from([("1".to_string(), ("woke".to_string(), Some(99.0)))]);
        let out = record(Some(&store), &mut state, woke, at);
        assert!(!out[0].notify);
        assert_eq!(state["1"].last_wake_at, Some(50.0));
        assert_eq!(state["1"].next_due, Some(99.0));
        assert_eq!(store.recent_events(10).len(), 2);
    }
}
