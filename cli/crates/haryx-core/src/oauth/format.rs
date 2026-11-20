//! Reset-time parsing and formatting: countdowns, local clock strings, ISO stamps.

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use serde_json::Value;

/// Why a reset time could not be formatted. The names are the Python exception types upstream
/// raised, which surface as a usage-fetch error kind when a response carries a bad timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    /// A string that is not an ISO 8601 date-time.
    ValueError,
    /// Not a string, or a date-time without an offset (naive), which cannot be compared to now.
    TypeError,
}

impl TimeError {
    pub fn kind(self) -> &'static str {
        match self {
            TimeError::ValueError => "ValueError",
            TimeError::TypeError => "TypeError",
        }
    }
}

/// Parse an ISO 8601 date-time carrying an offset (or `Z`).
pub(crate) fn parse_aware(s: &str) -> Result<Timestamp, TimeError> {
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Ok(ts);
    }
    match s.parse::<DateTime>() {
        Ok(_) => Err(TimeError::TypeError), // naive: no offset to compare against
        Err(_) => Err(TimeError::ValueError),
    }
}

/// Epoch seconds of an ISO string; a naive one is read as local time (as Python's
/// `datetime.fromisoformat(...).timestamp()` does).
pub fn parse_epoch(s: &str) -> Option<f64> {
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Some(ts_to_f64(ts));
    }
    let naive = s.parse::<DateTime>().ok()?;
    naive
        .to_zoned(TimeZone::system())
        .ok()
        .map(|z| ts_to_f64(z.timestamp()))
}

pub(crate) fn ts_to_f64(ts: Timestamp) -> f64 {
    ts.as_microsecond() as f64 / 1e6
}

pub(crate) fn f64_to_ts(epoch_s: f64) -> Option<Timestamp> {
    if !epoch_s.is_finite() {
        return None;
    }
    let micros = (epoch_s * 1e6).round();
    if micros.abs() > i64::MAX as f64 {
        return None;
    }
    Timestamp::from_microsecond(micros as i64).ok()
}

/// Python's `datetime.fromtimestamp(ts, tz=timezone.utc).isoformat()`:
/// `2026-07-05T08:59:00+00:00`, with `.ffffff` only when there are microseconds.
pub fn iso_utc(epoch_s: f64) -> Option<String> {
    let ts = f64_to_ts(epoch_s)?;
    let z = ts.to_zoned(TimeZone::UTC);
    let micros = z.subsec_nanosecond() / 1000;
    let base = z.strftime("%Y-%m-%dT%H:%M:%S").to_string();
    Some(if micros == 0 {
        format!("{base}+00:00")
    } else {
        format!("{base}.{micros:06}+00:00")
    })
}

/// `2026-07-05T08:59:00Z`: seconds precision, `Z` suffix (fraction truncated).
pub(crate) fn iso_utc_seconds_z(epoch_s: f64) -> Option<String> {
    let ts = Timestamp::from_second(epoch_s.floor() as i64).ok()?;
    Some(
        ts.to_zoned(TimeZone::UTC)
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
    )
}

/// `"5d 12h"` / `"1h 59m"` / `"45m"`: the grammar of every countdown shown.
pub fn format_countdown(seconds: f64) -> String {
    let total = if seconds.is_finite() {
        seconds.trunc().max(0.0) as i64
    } else {
        0
    };
    let days = total / 86400;
    let hours = (total % 86400) / 3600;
    let minutes = (total % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// Absolute reset time in `tz`: `"20:39"` same-day, else `"Jul 5 08:59"`.
pub fn reset_clock_string_in(reset: Timestamp, now: Timestamp, tz: &TimeZone) -> String {
    let reset_local: Zoned = reset.to_zoned(tz.clone());
    let now_local: Zoned = now.to_zoned(tz.clone());
    if reset_local.date() == now_local.date() {
        return reset_local.strftime("%H:%M").to_string();
    }
    format!(
        "{} {} {}",
        reset_local.strftime("%b"),
        reset_local.day(),
        reset_local.strftime("%H:%M")
    )
}

/// [`reset_clock_string_in`] in the system time zone.
pub fn reset_clock_string(reset: Timestamp, now: Timestamp) -> String {
    reset_clock_string_in(reset, now, &TimeZone::system())
}

/// `(countdown, clock)` for a reset time, against an explicit clock and zone.
pub fn format_reset_in(
    resets_at: &str,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<(String, String), TimeError> {
    let reset = parse_aware(resets_at)?;
    let seconds = ts_to_f64(reset) - ts_to_f64(now);
    Ok((
        format_countdown(seconds),
        reset_clock_string_in(reset, now, tz),
    ))
}

/// `(countdown, clock)` for a reset time in local time.
pub fn format_reset(resets_at: &str) -> Result<(String, String), TimeError> {
    format_reset_in(resets_at, Timestamp::now(), &TimeZone::system())
}

/// [`format_reset`] for a JSON value: a non-string is a `TypeError`, as upstream.
pub(crate) fn format_reset_value(
    resets_at: &Value,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<(String, String), TimeError> {
    match resets_at {
        Value::String(s) => format_reset_in(s, now, tz),
        _ => Err(TimeError::TypeError),
    }
}

/// `(countdown, clock)` for one usage window, or `None` when unknown.
///
/// Recomputed from `resets_at` at render time: strings cached at fetch time drift as the
/// measurement ages (a countdown frozen 2h ago overstates the wait by those 2h, and a same-day
/// "15:30" silently starts meaning yesterday). Entries persisted without a usable `resets_at` fall
/// back to the fetch-time strings — stale beats blank.
pub fn fresh_reset_strings(window: &Value) -> Option<(String, String)> {
    fresh_reset_strings_in(window, Timestamp::now(), &TimeZone::system())
}

/// [`fresh_reset_strings`] against an explicit clock and zone.
pub fn fresh_reset_strings_in(
    window: &Value,
    now: Timestamp,
    tz: &TimeZone,
) -> Option<(String, String)> {
    let obj = window.as_object()?;
    if let Some(resets_at) = obj.get("resets_at").filter(|v| super::truthy(v)) {
        if let Ok(pair) = format_reset_value(resets_at, now, tz) {
            return Some(pair);
        }
    }
    let clock = obj.get("clock")?;
    let countdown = obj.get("countdown").map_or_else(|| "?".to_string(), py_str);
    Some((countdown, py_str(clock)))
}

/// `str(value)` for the strings a window caches.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
