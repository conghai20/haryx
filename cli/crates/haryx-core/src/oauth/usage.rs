//! Normalising usage responses and reading the windows that gate an account.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use super::format::format_reset_value;
use super::{FetchError, py_float, py_num, truthy};

/// Marks a usage object read from rate-limit headers (`usage["source"]`). The headers can omit a
/// window the endpoint reports, so such a reading gates decisions only when it is complete (see
/// [`relevant_windows`]).
pub const USAGE_SOURCE_HEADERS: &str = "headers";

/// Label of the 5-hour window as [`relevant_windows`] reports it. The only window that is NOT
/// weekly: every consumer that treats the 5h window differently keys on this constant.
pub const FIVE_HOUR_LABEL: &str = "5h";

/// Label of the window a refused header probe adds (see [`relevant_windows`]).
pub const REJECTED_LABEL: &str = "rejected";

/// A normalised usage reading, as stored and printed (`five_hour`, `seven_day`, `spend`, `scoped`,
/// and for header readings `source` / `rejected`). Kept as JSON so the on-disk and `--json` shapes
/// match upstream byte for byte in their keys.
pub type Usage = Map<String, Value>;

/// One `(label, pct, resets_at)` window that gates an account.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub label: String,
    pub pct: f64,
    /// The ISO string as fetched, or `None` when the API sent none.
    pub resets_at: Option<String>,
}

impl UsageWindow {
    pub fn new(label: impl Into<String>, pct: f64, resets_at: Option<&str>) -> Self {
        UsageWindow {
            label: label.into(),
            pct,
            resets_at: resets_at.map(str::to_string),
        }
    }
}

/// The raw `five_hour`/`seven_day` window, or `None` when absent.
///
/// Absent means missing or `null`: from the usage endpoint, a limit the account does not have (a
/// verified Ultra account reports `five_hour: null`); from the header probe, a window it did not
/// carry. `{}` counts as absent too. Any other non-object value is a malformed response.
fn reported_window<'a>(
    data: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a Map<String, Value>>, FetchError> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(w)) => Ok((!w.is_empty()).then_some(w)),
        Some(_) => Err(FetchError::InvalidUsage(format!(
            "{key} must be an object or null"
        ))),
    }
}

fn time_error(e: super::format::TimeError) -> FetchError {
    FetchError::Other {
        kind: e.kind().to_string(),
        detail: "unusable resets_at".to_string(),
    }
}

/// Normalise one reported window to `{"pct", "resets_at", "countdown", "clock"}`.
///
/// `utilization: null` keeps the window with `pct` null (unavailable rather than zero). A missing,
/// boolean, string, non-finite or negative utilization fails the reading: dropping the window
/// would let the other one pass as a complete reading.
fn window_entry(
    key: &str,
    window: &Map<String, Value>,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<Value, FetchError> {
    let Some(pct) = window.get("utilization") else {
        return Err(FetchError::InvalidUsage(format!(
            "{key}.utilization is missing"
        )));
    };
    let valid = match pct {
        Value::Null => true,
        Value::Number(n) => n.as_f64().is_some_and(|f| f.is_finite() && f >= 0.0),
        _ => false,
    };
    if !valid {
        return Err(FetchError::InvalidUsage(format!(
            "{key}.utilization must be null or a finite non-negative number"
        )));
    }
    let mut entry = Map::new();
    entry.insert("pct".into(), pct.clone());
    add_reset(&mut entry, window.get("resets_at"), now, tz)?;
    Ok(Value::Object(entry))
}

/// Copy a truthy `resets_at` and its fetch-time countdown/clock into `entry`.
fn add_reset(
    entry: &mut Map<String, Value>,
    resets_at: Option<&Value>,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<(), FetchError> {
    let Some(resets_at) = resets_at.filter(|v| truthy(v)) else {
        return Ok(());
    };
    let (countdown, clock) = format_reset_value(resets_at, now, tz).map_err(time_error)?;
    entry.insert("resets_at".into(), resets_at.clone());
    entry.insert("countdown".into(), countdown.into());
    entry.insert("clock".into(), clock.into());
    Ok(())
}

fn f64_value(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
}

/// The pay-as-you-go spend row, or `None` when any figure it needs is null or unusable.
///
/// Claude Code returns nullable `used_credits`, `monthly_limit` and `utilization`
/// (`monthly_limit` null = unlimited); all three are needed to render the spend line, so when any
/// is missing just that row is skipped.
fn spend_entry(eu: &Map<String, Value>, now: Timestamp, tz: &TimeZone) -> Option<Value> {
    let field = |k: &str| eu.get(k).filter(|v| !v.is_null());
    let (used, limit, pct) = (
        field("used_credits")?,
        field("monthly_limit")?,
        field("utilization")?,
    );
    let mut entry = Map::new();
    entry.insert("used".into(), f64_value(py_float(used)? / 100.0));
    entry.insert("limit".into(), f64_value(py_float(limit)? / 100.0));
    entry.insert("pct".into(), f64_value(py_float(pct)?));
    entry.insert(
        "currency".into(),
        eu.get("currency").cloned().unwrap_or_else(|| json!("USD")),
    );
    if let Err(e) = add_reset(&mut entry, eu.get("resets_at"), now, tz) {
        log::debug!("extra_usage parse failed: {e:?}");
        return None;
    }
    Some(Value::Object(entry))
}

/// Per-model weekly limits from the newer `limits` array (`weekly_scoped` entries carrying a
/// `scope.model.display_name`, e.g. "Fable").
fn scoped_entries(
    limits: &[Value],
    now: Timestamp,
    tz: &TimeZone,
) -> Result<Vec<Value>, FetchError> {
    let mut scoped = Vec::new();
    for lim in limits.iter().filter_map(Value::as_object) {
        let name = lim
            .get("scope")
            .and_then(Value::as_object)
            .and_then(|s| s.get("model"))
            .and_then(Value::as_object)
            .and_then(|m| m.get("display_name"));
        let (Some(name), Some(pct)) = (
            name.filter(|n| truthy(n)),
            lim.get("percent").and_then(py_num),
        ) else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("name".into(), name.clone());
        entry.insert("pct".into(), f64_value(pct));
        add_reset(&mut entry, lim.get("resets_at"), now, tz)?;
        if let Some(Value::String(p)) = lim.get("probe_after") {
            // Only the header probe sets it (see the probe's `scoped_limit_from_headers`).
            entry.insert("probe_after".into(), Value::String(p.clone()));
        }
        scoped.push(Value::Object(entry));
    }
    Ok(scoped)
}

/// Normalise raw usage API data into the structure used by the CLI.
///
/// `five_hour`/`seven_day`: an absent window stays absent, `utilization: null` keeps the window
/// with `pct` null, and a malformed one fails the whole fetch as `bad-response`. `Ok(None)` when
/// the response carried nothing to show.
pub fn build_usage_result(data: &Value) -> Result<Option<Usage>, FetchError> {
    build_usage_result_in(data, Timestamp::now(), &TimeZone::system())
}

/// [`build_usage_result`] against an explicit clock and zone (for the fetch-time strings).
pub fn build_usage_result_in(
    data: &Value,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<Option<Usage>, FetchError> {
    log::debug!(
        "Usage API response: {}",
        serde_json::to_string_pretty(data).unwrap_or_default()
    );
    let Some(data) = data.as_object() else {
        return Err(FetchError::Other {
            kind: "AttributeError".into(),
            detail: "usage response is not an object".into(),
        });
    };
    let mut result = Map::new();
    for key in ["five_hour", "seven_day"] {
        if let Some(window) = reported_window(data, key)? {
            result.insert(key.into(), window_entry(key, window, now, tz)?);
        }
    }
    if let Some(eu) = data.get("extra_usage").filter(|v| truthy(v)) {
        let Some(eu) = eu.as_object() else {
            return Err(FetchError::Other {
                kind: "AttributeError".into(),
                detail: "extra_usage is not an object".into(),
            });
        };
        if eu.get("is_enabled").is_some_and(truthy)
            && let Some(spend) = spend_entry(eu, now, tz)
        {
            result.insert("spend".into(), spend);
        }
    }
    if let Some(Value::Array(limits)) = data.get("limits") {
        let scoped = scoped_entries(limits, now, tz)?;
        if !scoped.is_empty() {
            result.insert("scoped".into(), Value::Array(scoped));
        }
    }
    Ok((!result.is_empty()).then_some(result))
}

fn lower_set(models: &[String]) -> Vec<String> {
    models.iter().map(|m| m.to_lowercase()).collect()
}

/// Whether a header-probe reading carries every window a decision needs.
///
/// The 7d window always, plus each configured per-model window; the headers only carry Fable's,
/// and only with `usage.fableProbe` on, so `all` can never be shown complete. A missing 5h window
/// is not a gap: an Ultra account has none.
fn header_reading_complete(usage: &Map<String, Value>, models: &[String]) -> bool {
    if !usage.get("seven_day").is_some_and(Value::is_object) {
        return false;
    }
    if models.is_empty() {
        return true;
    }
    let wanted = lower_set(models);
    if wanted.iter().any(|m| m == "all") {
        return false;
    }
    let have: Vec<String> = usage
        .get("scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("name").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect();
    wanted.iter().all(|w| have.contains(w))
}

/// Every window that gates this account.
///
/// The reported 5-hour (`"5h"`) and 7-day (`"7d"`) windows: an Ultra account has no 5-hour one,
/// and a window without a figure is skipped. When `models` is non-empty, each named per-model
/// weekly `scoped` window is included too (matched case-insensitively on display name; the
/// sentinel `all` matches every scoped window). The single canonical window source for decisions,
/// scheduling and reset math. `spend` is a separate axis and deliberately excluded.
///
/// A header-probe reading missing any window the decision needs yields none of its windows, so it
/// reads as unknown — one 5h window at 5% must not pass for 95% headroom. A refused probe
/// (`rejected`) adds a 100% `"rejected"` window resetting when the refusal ends.
pub fn relevant_windows(usage: Option<&Value>, models: &[String]) -> Vec<UsageWindow> {
    let Some(usage) = usage.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut windows = Vec::new();
    let from_headers = usage.get("source").and_then(Value::as_str) == Some(USAGE_SOURCE_HEADERS);
    if !from_headers || header_reading_complete(usage, models) {
        for (key, label) in [("five_hour", FIVE_HOUR_LABEL), ("seven_day", "7d")] {
            let Some(window) = usage.get(key).and_then(Value::as_object) else {
                continue;
            };
            if let Some(pct) = window.get("pct").and_then(py_num) {
                let resets_at = window.get("resets_at").and_then(Value::as_str);
                windows.push(UsageWindow::new(label, pct, resets_at));
            }
        }
        if !models.is_empty() {
            let wanted = lower_set(models);
            let match_all = wanted.iter().any(|m| m == "all");
            let scoped = usage
                .get("scoped")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for s in scoped.filter_map(Value::as_object) {
                let (Some(pct), Some(name)) = (
                    s.get("pct").and_then(py_num),
                    s.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if match_all || wanted.contains(&name.to_lowercase()) {
                    let resets_at = s.get("resets_at").and_then(Value::as_str);
                    windows.push(UsageWindow::new(name, pct, resets_at));
                }
            }
        }
    }
    if let Some(Value::Object(rejected)) = usage.get("rejected") {
        let resets_at = rejected.get("resets_at").and_then(Value::as_str);
        windows.push(UsageWindow::new(REJECTED_LABEL, 100.0, resets_at));
    }
    windows
}

/// Remaining percentage before this account hits a rate-limit window: `100 - max(pct)` over
/// [`relevant_windows`], so `<= 0` means at or over a limit. `None` when usage is unavailable or
/// carries no window data, which callers treat as "unknown" (never auto-skipped).
pub fn account_headroom(usage: Option<&Value>, models: &[String]) -> Option<f64> {
    relevant_windows(usage, models)
        .iter()
        .map(|w| w.pct)
        .reduce(f64::max)
        .map(|max| 100.0 - max)
}
