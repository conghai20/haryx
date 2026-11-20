//! The header probe: usage read from the `anthropic-ratelimit-unified-*` headers of a minimal
//! model request, for inference-only setup-tokens the usage endpoint will not answer.

use std::time::Duration;

use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::fetch::UsageOutcome;
use super::format::{format_reset_value, iso_utc, ts_to_f64};
use super::http::{self, FetchError, Headers};
use super::usage::{USAGE_SOURCE_HEADERS, build_usage_result};
use super::{
    Api, OAUTH_BETA_HEADER, USAGE_PROBE_API_VERSION, USAGE_PROBE_MODEL, USAGE_PROBE_SCOPED_MODELS,
    USAGE_PROBE_SYSTEM, USER_AGENT, py_num,
};
use crate::poll_policy;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Unified rate-limit header prefix → usage API window key.
const UNIFIED_WINDOWS: &[(&str, &str)] = &[("5h", "five_hour"), ("7d", "seven_day")];

/// Per-model weekly buckets by header prefix: the display name the usage API's `weekly_scoped`
/// limits use, and the representative claim the headers give when that bucket is the limit their
/// status is about. `7d_oi` is Fable's weekly window (Claude Code 2.1.322 labels its claim the
/// "Fable limit").
const UNIFIED_SCOPED_WINDOWS: &[(&str, &str, &str)] =
    &[("7d_oi", "Fable", "seven_day_overage_included")];

fn scoped_window(prefix: &str) -> Option<(&'static str, &'static str)> {
    UNIFIED_SCOPED_WINDOWS
        .iter()
        .find(|(p, _, _)| *p == prefix)
        .map(|(_, n, c)| (*n, *c))
}

/// Python's `round(x, 2)`: correctly rounded on the exact binary value, ties to even.
fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// `(utilization %, resets_at ISO)` of one unified window, or `None` when its utilization is
/// missing or unusable (a missing reset only drops `resets_at`).
fn unified_window(headers: &Headers, prefix: &str) -> Option<(f64, Option<String>)> {
    let base = format!("anthropic-ratelimit-unified-{prefix}");
    let fraction: f64 = headers
        .get(&format!("{base}-utilization"))?
        .trim()
        .parse()
        .ok()?;
    if !fraction.is_finite() || fraction < 0.0 {
        return None;
    }
    Some((
        round2(fraction * 100.0),
        epoch_header_iso(headers.get(&format!("{base}-reset"))),
    ))
}

/// Whether any `anthropic-ratelimit-unified-<prefix>-*` header is present.
fn unified_window_present(headers: &Headers, prefix: &str) -> bool {
    let start = format!("anthropic-ratelimit-unified-{prefix}-");
    headers.keys().any(|k| k.starts_with(&start))
}

/// An epoch-seconds header value as ISO 8601, or `None` when unusable.
fn epoch_header_iso(raw: Option<&str>) -> Option<String> {
    let ts: f64 = raw?.trim().parse().ok()?;
    (ts.is_finite() && ts > 0.0).then(|| iso_utc(ts)).flatten()
}

fn num(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
}

/// One per-model weekly limit in the usage API's `limits` shape. `probe_after` is the probe's own
/// addition for a bucket it read as spent.
fn scoped_limit(name: &str, pct: f64, resets_at: Option<&str>, probe_after: Option<&str>) -> Value {
    let mut limit = Map::new();
    limit.insert("kind".into(), json!("weekly_scoped"));
    limit.insert("scope".into(), json!({"model": {"display_name": name}}));
    limit.insert("percent".into(), num(pct));
    if let Some(r) = resets_at.filter(|r| !r.is_empty()) {
        limit.insert("resets_at".into(), json!(r));
    }
    if let Some(p) = probe_after.filter(|p| !p.is_empty()) {
        limit.insert("probe_after".into(), json!(p));
    }
    Value::Object(limit)
}

/// Map `anthropic-ratelimit-unified-*` headers to the raw usage API shape, so
/// [`build_usage_result`] normalises them unchanged.
///
/// `-utilization` is a 0-1 fraction (the API's `utilization` is a percentage) and `-reset` is
/// epoch seconds (the API's `resets_at` is ISO 8601). A 5h or 7d window with no header at all is
/// left out; one whose headers are present without a usable utilization is kept unusable so the
/// reading fails. Fable's `7d_oi` bucket becomes a `weekly_scoped` limit named "Fable". A
/// `rejected` unified status adds a `rejected` entry.
pub fn parse_unified_ratelimit_headers(headers: &Headers) -> Map<String, Value> {
    let mut data = Map::new();
    for (prefix, key) in UNIFIED_WINDOWS {
        match unified_window(headers, prefix) {
            Some((pct, resets_at)) => {
                let mut w = Map::new();
                w.insert("utilization".into(), num(pct));
                if let Some(r) = resets_at {
                    w.insert("resets_at".into(), json!(r));
                }
                data.insert((*key).into(), Value::Object(w));
            }
            // Unusable: a non-number utilization fails the reading as bad-response.
            None if unified_window_present(headers, prefix) => {
                data.insert((*key).into(), json!({"utilization": "NaN"}));
            }
            None => {}
        }
    }
    let limits: Vec<Value> = UNIFIED_SCOPED_WINDOWS
        .iter()
        .filter_map(|(prefix, name, _)| {
            let (pct, resets_at) = unified_window(headers, prefix)?;
            Some(scoped_limit(name, pct, resets_at.as_deref(), None))
        })
        .collect();
    if !limits.is_empty() {
        data.insert("limits".into(), Value::Array(limits));
    }
    if headers.get("anthropic-ratelimit-unified-status") == Some("rejected") {
        let reset = epoch_header_iso(headers.get("anthropic-ratelimit-unified-reset"));
        data.insert("rejected".into(), json!({"resets_at": reset}));
    }
    data
}

fn is_refusal(headers: &Headers) -> bool {
    headers.get("anthropic-ratelimit-unified-status") == Some("rejected")
}

impl Api {
    /// A 1-output-token `/v1/messages` request to `model`.
    fn probe(&self, access_token: &str, model: &str) -> Result<http::HttpResponse, FetchError> {
        let body = json!({
            "model": model,
            "max_tokens": 1,
            "system": USAGE_PROBE_SYSTEM,
            "messages": [{"role": "user", "content": "hi"}],
        });
        let auth = format!("Bearer {access_token}");
        let headers = [
            ("Authorization", auth.as_str()),
            ("anthropic-beta", OAUTH_BETA_HEADER),
            ("anthropic-version", USAGE_PROBE_API_VERSION),
            ("Content-Type", "application/json"),
            ("User-Agent", USER_AGENT),
        ];
        http::post(
            &self.messages_url,
            &headers,
            body.to_string().as_bytes(),
            PROBE_TIMEOUT,
        )
    }

    /// Read utilization from the rate-limit headers of a minimal model request.
    ///
    /// A 429 carrying `anthropic-ratelimit-unified-status: rejected` is the account confirming
    /// its quota refuses inference whatever the numbers say, so it is returned as a reading with
    /// a `rejected` entry: `{"retry_after_s", "resets_at"}` (now + Retry-After, else the unified
    /// reset, else null). A bare 429 proves no exhaustion and stays an `http-429` failure.
    pub fn request_usage_via_probe(
        &self,
        access_token: &str,
    ) -> Result<Map<String, Value>, FetchError> {
        match self.probe(access_token, USAGE_PROBE_MODEL) {
            Ok(resp) => Ok(parse_unified_ratelimit_headers(&resp.headers)),
            Err(e) => {
                let FetchError::Http {
                    status: 429,
                    headers,
                    ..
                } = &e
                else {
                    return Err(e);
                };
                let mut data = parse_unified_ratelimit_headers(headers);
                let Some(Value::Object(mut rejected)) = data.remove("rejected") else {
                    return Err(e);
                };
                let (_, retry_after) = e.classify();
                if let Some(wait) = retry_after {
                    let until = iso_utc(ts_to_f64(Timestamp::now()) + wait);
                    rejected.insert("resets_at".into(), json!(until));
                }
                rejected.insert("retry_after_s".into(), retry_after.map_or(Value::Null, num));
                data.insert("rejected".into(), Value::Object(rejected));
                Ok(data)
            }
        }
    }

    /// One 1-token request to `model`, read as its weekly bucket. A served response and a 429
    /// with a `rejected` unified status both describe the bucket; anything else says nothing.
    fn probe_scoped_limit(
        &self,
        token: &str,
        model: &str,
        prefix: &str,
        now: f64,
    ) -> Option<Value> {
        match self.probe(token, model) {
            Ok(resp) => scoped_limit_from_headers(&resp.headers, prefix, None, now),
            Err(e) => match &e {
                FetchError::Http {
                    status: 429,
                    headers,
                    ..
                } if is_refusal(headers) => {
                    scoped_limit_from_headers(headers, prefix, e.classify().1, now)
                }
                FetchError::Http { status, .. } => {
                    log::debug!("Scoped usage probe ({model}) failed: HTTP {status}");
                    None
                }
                other => {
                    log::debug!("Scoped usage probe ({model}) failed: {other}");
                    None
                }
            },
        }
    }

    /// Add each per-model weekly bucket the main probe lacks. No request is sent for a bucket the
    /// main response already carries, one whose stored row is still paused (that row is carried
    /// over as it is), or when the main probe says `rejected` (a second request only adds load or
    /// spends usage credits).
    fn add_scoped_limits(
        &self,
        data: &mut Map<String, Value>,
        token: &str,
        last_usage: Option<&Value>,
    ) {
        let mut limits: Vec<Value> = match data.get("limits") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        };
        let mut have: Vec<Value> = limits.iter().filter_map(display_name).cloned().collect();
        let now = ts_to_f64(Timestamp::now());
        for (model, prefix) in USAGE_PROBE_SCOPED_MODELS {
            let Some((name, _)) = scoped_window(prefix) else {
                continue;
            };
            if have.iter().any(|h| h.as_str() == Some(name)) {
                continue;
            }
            let mut limit = paused_scoped_limit(last_usage, name, now);
            if let Some(l) = &limit {
                log::debug!(
                    "Scoped usage probe ({model}) paused until {}: keeping the stored row",
                    l.get("probe_after").and_then(Value::as_str).unwrap_or("?")
                );
            } else if !data.contains_key("rejected") {
                limit = self.probe_scoped_limit(token, model, prefix, now);
            }
            if let Some(l) = limit {
                have.push(json!(name));
                limits.push(l);
            }
        }
        if !limits.is_empty() {
            data.insert("limits".into(), Value::Array(limits));
        }
    }

    /// Usage from the header probe, classified like an endpoint fetch.
    ///
    /// The result is marked `source: headers` and keeps the probe's `rejected` entry, so a
    /// refused probe is stored (and decided on) as an account at its limit until the refusal
    /// ends. That rejection outranks a window that fails to normalise: the reading then keeps
    /// `rejected` and its per-model rows, and drops the account's windows.
    pub(crate) fn usage_from_probe(
        &self,
        token: &str,
        context: &str,
        scoped_probe: bool,
        last_usage: Option<&Value>,
    ) -> UsageOutcome {
        let mut data = match self.request_usage_via_probe(token) {
            Ok(data) => data,
            Err(e) => {
                let (kind, retry_after) = e.classify();
                let cause = match retry_after {
                    Some(r) => format!("{kind}, retry-after {r:.0}s"),
                    None => kind.clone(),
                };
                log::warn!("Usage header probe failed {context}: {cause}");
                log::debug!("Usage header probe failure detail {context}: {e:?}");
                return UsageOutcome::failed(&kind, retry_after);
            }
        };
        if scoped_probe {
            self.add_scoped_limits(&mut data, token, last_usage);
        }
        let mut usage = match build_usage_result(&Value::Object(data.clone())) {
            Ok(u) => u.unwrap_or_default(),
            Err(e) => {
                let kind = e.kind();
                log::debug!("Usage header probe failure detail {context}: {e:?}");
                if !(matches!(e, FetchError::InvalidUsage(_)) && data.contains_key("rejected")) {
                    log::warn!("Usage header probe failed {context}: {kind}");
                    return UsageOutcome::failed(&kind, None);
                }
                log::warn!("Usage header probe {context}: {kind}, keeping the rejection");
                let limits = data.get("limits").cloned().unwrap_or(Value::Null);
                match build_usage_result(&json!({"limits": limits})) {
                    Ok(u) => u.unwrap_or_default(),
                    Err(e) => return UsageOutcome::failed(&e.kind(), None),
                }
            }
        };
        if let Some(rejected) = data.get("rejected") {
            usage.insert("rejected".into(), rejected.clone());
            let wait = rejected
                .get("retry_after_s")
                .map_or("None".to_string(), Value::to_string);
            log::warn!("Usage header probe {context}: request rejected (retry-after {wait})");
        }
        if usage.is_empty() {
            log::warn!("Usage header probe {context}: response carried no rate-limit headers");
            return UsageOutcome::failed("bad-response", None);
        }
        usage.insert("source".into(), json!(USAGE_SOURCE_HEADERS));
        UsageOutcome::ok(Some(usage))
    }
}

fn display_name(limit: &Value) -> Option<&Value> {
    limit.get("scope")?.get("model")?.get("display_name")
}

/// The stored row for bucket `name` as a raw limit while its probe is paused, else `None`. A row
/// whose reset cannot be read is not kept.
fn paused_scoped_limit(last_usage: Option<&Value>, name: &str, now: f64) -> Option<Value> {
    let rows = last_usage?.get("scoped")?.as_array()?;
    let row = rows.iter().find(|row| {
        row.is_object()
            && row.get("name").and_then(Value::as_str) == Some(name)
            && row.get("pct").and_then(py_num).is_some()
            && poll_policy::scoped_probe_paused(row, now)
    })?;
    let resets_at = row.get("resets_at").filter(|v| super::truthy(v));
    if let Some(r) = resets_at {
        let tz = jiff::tz::TimeZone::system();
        format_reset_value(r, Timestamp::now(), &tz).ok()?;
    }
    let pct = row.get("pct").and_then(py_num)?;
    let probe_after = row.get("probe_after").and_then(Value::as_str);
    Some(scoped_limit(
        name,
        pct,
        resets_at.and_then(Value::as_str),
        probe_after,
    ))
}

/// The per-model bucket a scoped probe's response reports, or `None` when it reports none.
///
/// The bucket is spent when the unified status is `rejected` for it, when the bucket's own
/// `-status` is, or when its utilization is at 100% (as Claude Code 2.1.322 reads these headers,
/// a served response with a `rejected` unified status ran on usage credits). A spent bucket:
///
/// - reads as at least 100%, whatever its utilization header says;
/// - resets at its own reset, else at the unified reset when the unified status marked it;
/// - carries `probe_after` ([`poll_policy::scoped_probe_after`]), before which it is not probed
///   again. A refusal's Retry-After moves only that deadline.
///
/// The unified status is this bucket's only when the representative claim names it or there is
/// no claim; one naming another limit leaves the bucket read from its own headers alone.
pub(crate) fn scoped_limit_from_headers(
    headers: &Headers,
    prefix: &str,
    retry_after_s: Option<f64>,
    now: f64,
) -> Option<Value> {
    let (name, claim) = scoped_window(prefix)?;
    let base = format!("anthropic-ratelimit-unified-{prefix}");
    let pct = unified_window(headers, prefix).map(|(p, _)| p);
    let mut resets_at = epoch_header_iso(headers.get(&format!("{base}-reset")));
    let representative = headers.get("anthropic-ratelimit-unified-representative-claim");
    let past_limit = is_refusal(headers) && representative.is_none_or(|c| c == claim);
    let spent = past_limit
        || headers.get(&format!("{base}-status")) == Some("rejected")
        || pct.is_some_and(|p| p >= 100.0);
    if !spent {
        return pct.map(|p| scoped_limit(name, p, resets_at.as_deref(), None));
    }
    let retry_after_s = if past_limit { retry_after_s } else { None };
    if past_limit && resets_at.is_none() {
        resets_at = epoch_header_iso(headers.get("anthropic-ratelimit-unified-reset"));
    }
    let until = poll_policy::scoped_probe_after(now, resets_at.as_deref(), retry_after_s);
    let probe_after = iso_utc(until);
    Some(scoped_limit(
        name,
        pct.unwrap_or(0.0).max(100.0),
        resets_at.as_deref(),
        probe_after.as_deref(),
    ))
}
