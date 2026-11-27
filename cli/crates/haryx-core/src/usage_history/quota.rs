//! 5-hour and weekly readings over time (`usage/quota.jsonl`), appended whenever the usage store
//! records a new measurement. Unchanged readings are skipped.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use super::{QUOTA, history_dir};

/// One reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub at: i64,
    pub account: i64,
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

fn pct(usage: &Value, window: &str) -> Option<f64> {
    usage.get(window)?.get("pct")?.as_f64()
}

/// Append the readings in `rows` (usage-store rows: `lastGood`, `fetchedAt`) that differ from
/// each account's last one. Best effort.
pub fn record(store_root: &Path, rows: &[(i64, &Value)]) {
    if rows.is_empty() || !crate::settings::load_history_settings(store_root).enabled {
        return;
    }
    let readings: Vec<Reading> = rows
        .iter()
        .filter_map(|(account, row)| {
            let good = row.get("lastGood")?;
            Some(Reading {
                at: row.get("fetchedAt").and_then(Value::as_f64)? as i64,
                account: *account,
                five_hour: pct(good, "five_hour"),
                seven_day: pct(good, "seven_day"),
            })
        })
        .collect();
    let dir = history_dir(store_root);
    let last = latest(&dir);
    let fresh: Vec<&Reading> = readings
        .iter()
        .filter(|r| {
            last.get(&r.account).is_none_or(|l| {
                (l.five_hour, l.seven_day) != (r.five_hour, r.seven_day) && r.at > l.at
            })
        })
        .collect();
    if fresh.is_empty() || std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(QUOTA))
    else {
        return;
    };
    for r in fresh {
        let row = json!({"at": r.at, "account": r.account, "fiveHour": r.five_hour, "sevenDay": r.seven_day});
        let _ = writeln!(file, "{row}");
    }
}

/// Every reading, oldest first.
pub fn read(dir: &Path) -> Vec<Reading> {
    let Ok(text) = std::fs::read_to_string(dir.join(QUOTA)) else {
        return Vec::new();
    };
    let mut out: Vec<Reading> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            Some(Reading {
                at: v.get("at")?.as_i64()?,
                account: v.get("account")?.as_i64()?,
                five_hour: v.get("fiveHour").and_then(Value::as_f64),
                seven_day: v.get("sevenDay").and_then(Value::as_f64),
            })
        })
        .collect();
    out.sort_by_key(|r| r.at);
    out
}

fn latest(dir: &Path) -> HashMap<i64, Reading> {
    read(dir).into_iter().map(|r| (r.account, r)).collect()
}

/// Drop readings older than `cutoff` (unix seconds).
pub fn prune(dir: &Path, cutoff: i64) {
    let all = read(dir);
    if all.first().is_none_or(|r| r.at >= cutoff) {
        return;
    }
    let kept: String = all
        .iter()
        .filter(|r| r.at >= cutoff)
        .map(|r| {
            json!({"at": r.at, "account": r.account, "fiveHour": r.five_hour, "sevenDay": r.seven_day})
                .to_string()
                + "\n"
        })
        .collect();
    let _ = crate::fsutil::atomic_write_text(
        &dir.join(QUOTA),
        &kept,
        crate::fsutil::WriteOptions::default(),
    );
}

/// How far back [`weekly_burn_per_s`] looks: recent enough to follow a change of habit, long
/// enough to span a night.
pub const BURN_WINDOW_S: i64 = 72 * 3600;
/// A shorter record than this says nothing about a rate.
pub const BURN_MIN_SPAN_S: i64 = 6 * 3600;

/// The whole fleet's weekly use per second over the last [`BURN_WINDOW_S`], in % of one account's
/// week: every rise of each account's weekly %, a fall counted as a reset (its new % is all new
/// use). `None` when the readings span less than [`BURN_MIN_SPAN_S`] of that window.
pub fn weekly_burn_per_s(readings: &[Reading], now: i64) -> Option<f64> {
    let since = now - BURN_WINDOW_S;
    let weekly: Vec<&Reading> = readings.iter().filter(|r| r.seven_day.is_some()).collect();
    let first = weekly.first()?.at;
    let mut last: HashMap<i64, f64> = HashMap::new();
    let mut used = 0.0;
    for r in &weekly {
        let pct = r.seven_day.unwrap_or_default();
        if let Some(prev) = last.insert(r.account, pct)
            && r.at >= since
        {
            used += if pct >= prev { pct - prev } else { pct };
        }
    }
    let span = now - first.max(since);
    (span >= BURN_MIN_SPAN_S).then(|| used / span as f64)
}

/// How many times each account reached its limit (a reading at 100% after one below it).
pub fn exhaustions(readings: &[Reading], since: i64) -> HashMap<i64, u32> {
    let mut last_full: HashMap<i64, bool> = HashMap::new();
    let mut out = HashMap::new();
    for r in readings {
        let full = r.five_hour.unwrap_or(0.0) >= 100.0 || r.seven_day.unwrap_or(0.0) >= 100.0;
        let was = last_full.insert(r.account, full).unwrap_or(false);
        if full && !was && r.at >= since {
            *out.entry(r.account).or_insert(0) += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(at: f64, five: f64) -> Value {
        json!({"fetchedAt": at, "lastGood": {"five_hour": {"pct": five}, "seven_day": {"pct": 10.0}}})
    }

    #[test]
    fn records_only_changes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (a, b, c) = (row(100.0, 20.0), row(200.0, 20.0), row(300.0, 55.0));
        record(tmp.path(), &[(1, &a)]);
        record(tmp.path(), &[(1, &b)]);
        record(tmp.path(), &[(1, &c), (2, &a)]);
        let all = read(&history_dir(tmp.path()));
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().filter(|r| r.account == 1).count(), 2);
    }

    #[test]
    fn counts_exhaustions_and_prunes() {
        let r = |at, five| Reading {
            at,
            account: 1,
            five_hour: Some(five),
            seven_day: None,
        };
        let readings = [
            r(10, 50.0),
            r(20, 100.0),
            r(30, 100.0),
            r(40, 20.0),
            r(50, 100.0),
        ];
        assert_eq!(exhaustions(&readings, 0).get(&1), Some(&2));
        assert_eq!(exhaustions(&readings, 45).get(&1), Some(&1));
    }

    fn weekly(at: i64, account: i64, pct: f64) -> Reading {
        Reading {
            at,
            account,
            five_hour: None,
            seven_day: Some(pct),
        }
    }

    #[test]
    fn burn_sums_rises_and_counts_a_reset_as_new_use() {
        let h = 3600;
        let now = 100 * h;
        let readings = [
            weekly(now - 10 * h, 1, 50.0),
            weekly(now - 5 * h, 1, 60.0),
            weekly(now - 4 * h, 2, 90.0),
            weekly(now - 2 * h, 2, 5.0),
        ];
        let burn = weekly_burn_per_s(&readings, now).unwrap();
        assert!((burn * 3600.0 - 1.5).abs() < 1e-9, "{burn}");
    }

    #[test]
    fn burn_needs_a_long_enough_record() {
        let h = 3600;
        let readings = [weekly(97 * h, 1, 10.0), weekly(99 * h, 1, 20.0)];
        assert_eq!(weekly_burn_per_s(&readings, 100 * h), None);
        assert_eq!(weekly_burn_per_s(&[], 100 * h), None);

        let tmp = tempfile::TempDir::new().unwrap();
        let (a, b) = (row(100.0, 20.0), row(200.0, 30.0));
        record(tmp.path(), &[(1, &a)]);
        record(tmp.path(), &[(1, &b)]);
        prune(&history_dir(tmp.path()), 150);
        assert_eq!(read(&history_dir(tmp.path())).len(), 1);
    }
}
