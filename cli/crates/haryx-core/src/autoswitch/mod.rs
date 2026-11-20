//! Port of `claude_swap/autoswitch.py`: the auto-switch engine behind `haryx auto`.
//!
//! One [`AutoSwitchEngine::tick`] reads usage (on the shared poll schedule), decides whether the
//! active account should hand over, picks a target by strategy, freshens its token and switches.
//! Every observation leaves as an [`Event`]; state that must survive restarts (last switch,
//! quarantine, health counters) lives in `autoswitch_state.json` in the store root.

mod engine;
pub mod events;
mod rank;
mod schedule;
mod tick;

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::lines::binding_window;
use crate::models::pyfmt::format_g_precision;
use crate::oauth;
use crate::poll_policy::{self, parse_reset_ts};
use crate::usage_store::{Decision, UsageEntry};

pub use engine::AutoSwitchEngine;
pub use events::{Event, EventBody};

pub const STATE_FILENAME: &str = "autoswitch_state.json";
pub const STATE_SCHEMA_VERSION: i64 = 1;

/// Freshen a target whose access token expires within this window.
pub const FRESHEN_BUFFER_MS: f64 = 10.0 * 60.0 * 1000.0;
/// Longest single sleep in the loop.
pub const MAX_SLEEP_S: f64 = poll_policy::EXHAUSTED_INTERVAL_S;
/// Blocked wait when no reset time is known.
pub const NO_RESET_FALLBACK_S: f64 = 300.0;
/// How long an expired-but-owned active token reads as "idle" before failover counting resumes.
pub const IDLE_HOLD_MAX_S: f64 = 30.0 * 60.0;
/// A left account counts as recovered only when it resets this much sooner.
pub const RECOVERY_HYSTERESIS_S: f64 = 300.0;
/// Failover snapshot: an infinite active recovery only loses to a peer recovering this soon.
pub const RECOVERY_HORIZON_S: f64 = 4.0 * 3600.0;
pub const HORIZON_HEADROOM_RATIO: f64 = 2.0;
pub const SPENT_HEADROOM_PCT: f64 = 3.0;

/// Systemic freshen statuses, most important first, with their user-facing explanation.
pub const SYSTEMIC_MESSAGES: [(&str, &str); 4] = [
    (
        "store-unmirrored",
        "CLAUDE_SECURESTORAGE_CONFIG_DIR is set — unset it or run haryx from a normal shell",
    ),
    (
        "invalid_client",
        "haryx's OAuth client was rejected — systemic, not this account",
    ),
    (
        "stash-unreadable",
        "a stashed successor is unreadable — unlock the keychain or fix the file, then retry; \
         `haryx unclaimed` inspects it",
    ),
    (
        "consume-busy",
        "another haryx surface holds the slot — retries next pass",
    ),
];

fn systemic_rank(status: &str) -> Option<usize> {
    SYSTEMIC_MESSAGES.iter().position(|(s, _)| *s == status)
}

fn systemic_message(status: &str) -> &'static str {
    SYSTEMIC_MESSAGES
        .iter()
        .find(|(s, _)| *s == status)
        .map_or("", |(_, m)| m)
}

/// Result of one tick; doubles as the `auto --once` exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickOutcome {
    Switched = 0,
    Error = 1,
    NoAction = 2,
    /// Wanted to switch but no viable target / all exhausted.
    Blocked = 3,
}

impl TickOutcome {
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// When the all-above escape lets a candidate win: it must beat the active account's headroom by
/// the smaller of doubling it or [`SPENT_HEADROOM_PCT`] points.
pub fn all_above_margin_met(candidate_headroom: f64, active_headroom: f64) -> bool {
    let required =
        (active_headroom * HORIZON_HEADROOM_RATIO).min(active_headroom + SPENT_HEADROOM_PCT);
    candidate_headroom >= required
}

/// `f"{value:.10g}"`.
pub fn pct_label(value: f64) -> String {
    format_g_precision(value, 10)
}

fn below_line_detail(
    binding: Option<&crate::lines::BindingWindow>,
    active_headroom: f64,
    threshold: f64,
) -> String {
    match binding {
        None => format!(
            "{}% < {}%",
            pct_label(100.0 - active_headroom),
            pct_label(threshold)
        ),
        Some(b) => format!("{} {}% < {}%", b.label, pct_label(b.pct), pct_label(b.line)),
    }
}

// ------------------------------------------------------------------------------- usage values

/// One account's decision value, owned: a usage dict or a sentinel string.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UsageVal {
    Dict(Value),
    Sentinel(String),
}

/// `account → decision value` (`None`: unknown), in slot order.
pub(crate) type UsageMap = Vec<(String, Option<UsageVal>)>;

pub(crate) fn usage_from_entries(entries: &BTreeMap<String, UsageEntry>) -> UsageMap {
    let mut out: UsageMap = entries
        .iter()
        .map(|(num, entry)| {
            let value = entry.decision_value().map(|d| match d {
                Decision::Sentinel(s) => UsageVal::Sentinel(s.to_owned()),
                Decision::Usage(v) => UsageVal::Dict(v.clone()),
            });
            (num.clone(), value)
        })
        .collect();
    out.sort_by_key(|(num, _)| crate::switcher::slot_key(num));
    out
}

/// The usage dict for `num`, if its decision value is one.
pub(crate) fn dict_of<'a>(usage: &'a UsageMap, num: &str) -> Option<&'a Value> {
    usage
        .iter()
        .find(|(n, _)| n == num)
        .and_then(|(_, v)| match v {
            Some(UsageVal::Dict(d)) => Some(d),
            _ => None,
        })
}

pub(crate) fn sentinel_of<'a>(usage: &'a UsageMap, num: &str) -> Option<&'a str> {
    usage
        .iter()
        .find(|(n, _)| n == num)
        .and_then(|(_, v)| match v {
            Some(UsageVal::Sentinel(s)) => Some(s.as_str()),
            _ => None,
        })
}

/// Each account's headroom (`None`: unknown), in `usage` order.
pub(crate) fn headroom_by_account(
    usage: &UsageMap,
    models: &[String],
) -> Vec<(String, Option<f64>)> {
    usage
        .iter()
        .map(|(num, _)| {
            (
                num.clone(),
                oauth::account_headroom(dict_of(usage, num), models),
            )
        })
        .collect()
}

pub(crate) fn headroom_of(headroom: &[(String, Option<f64>)], num: &str) -> Option<f64> {
    headroom
        .iter()
        .find(|(n, _)| n == num)
        .and_then(|(_, h)| *h)
}

/// `{label: pct}` of every relevant window.
pub(crate) fn window_pcts(usage: Option<&Value>, models: &[String]) -> Vec<(String, f64)> {
    oauth::relevant_windows(usage, models)
        .into_iter()
        .map(|w| (w.label, w.pct))
        .collect()
}

/// The 7-day window's reset, when known and in the future.
pub(crate) fn seven_day_reset_ts(usage: Option<&Value>, now: f64) -> Option<f64> {
    let at = usage?.get("seven_day")?.get("resets_at")?.as_str();
    parse_reset_ts(at).filter(|ts| *ts > now)
}

/// When the binding window resets (`inf` when unknown or past).
pub(crate) fn binding_recovery_ts(
    usage: Option<&Value>,
    models: &[String],
    now: f64,
    lines: Option<(f64, Option<f64>, f64)>,
) -> f64 {
    let windows = oauth::relevant_windows(usage, models);
    if windows.is_empty() {
        return f64::INFINITY;
    }
    let resets_at = match lines {
        Some((threshold, Some(weekly), scale)) => {
            binding_window(&windows, threshold, Some(weekly), Some(scale)).and_then(|b| b.resets_at)
        }
        _ => windows
            .iter()
            .fold(None::<&oauth::UsageWindow>, |best, w| match best {
                Some(b) if b.pct >= w.pct => Some(b),
                _ => Some(w),
            })
            .and_then(|w| w.resets_at.clone()),
    };
    parse_reset_ts(resets_at.as_deref())
        .filter(|ts| *ts > now)
        .unwrap_or(f64::INFINITY)
}

/// Per-account switch lines with a global default.
#[derive(Clone, Debug)]
pub(crate) struct ThresholdMap {
    overrides: HashMap<String, f64>,
    default: f64,
}

impl ThresholdMap {
    pub(crate) fn new(overrides: HashMap<String, f64>, default: f64) -> Self {
        ThresholdMap { overrides, default }
    }

    pub(crate) fn get(&self, num: &str) -> f64 {
        self.overrides.get(num).copied().unwrap_or(self.default)
    }
}

fn every_candidate_above_threshold(
    candidates: &[String],
    over: &HashMap<String, Option<bool>>,
) -> bool {
    let measured: Vec<bool> = candidates
        .iter()
        .filter_map(|n| over.get(n).copied().flatten())
        .collect();
    !measured.is_empty() && measured.iter().all(|b| *b)
}

fn every_account_above_threshold(
    candidates: &[String],
    over: &HashMap<String, Option<bool>>,
    active_over: Option<bool>,
) -> bool {
    active_over == Some(true) && every_candidate_above_threshold(candidates, over)
}

/// `{"number": int, "email": str}`.
pub(crate) fn account_ref(number: &str, email: &str) -> Value {
    let num = number
        .parse::<i64>()
        .map_or_else(|_| Value::from(number), Value::from);
    serde_json::json!({"number": num, "email": email})
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_engine;
