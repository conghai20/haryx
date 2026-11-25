//! OAuth token management and the usage API. Port of `claude_swap/oauth.py`.
//!
//! - credential helpers (access token, fingerprints, login expiry) — this file;
//! - the token refresh grant and the profile endpoint — [`refresh`];
//! - the usage endpoint, normalisation and the windows that gate an account — [`usage`], [`fetch`];
//! - the header probe for inference-only setup-tokens — [`probe`];
//! - reset-time formatting — [`format`].
//!
//! Every request goes through an [`Api`], which names the endpoints. [`Api::from_env`] uses the
//! real hosts unless `HARYX_API_BASE` (test-only) points all of them at one local server.
//! Network must never run while a credential/config lock is held, except the documented refresh.

mod fetch;
mod format;
pub(crate) mod http;
mod probe;
mod refresh;
mod usage;

use serde_json::Value;
use sha2::{Digest, Sha256};

pub use fetch::{
    DETERMINISTIC_REFRESH_ERRORS, PersistFn, RefreshViaFn, UsageFetch, UsageOutcome,
    try_fetch_usage_for_account,
};
pub use format::{
    TimeError, format_countdown, format_reset, format_reset_in, fresh_reset_strings,
    fresh_reset_strings_in, reset_clock_string, reset_clock_string_in,
};
pub use format::{iso_utc, parse_epoch as parse_iso_epoch};
pub use http::{FetchError, Headers, HttpResponse};
pub use probe::parse_unified_ratelimit_headers;
pub use refresh::{
    AccountIdentity, REFRESH_TIMEOUT, RefreshOutcome, fetch_oauth_profile,
    try_refresh_oauth_credentials,
};
pub use usage::{
    FIVE_HOUR_LABEL, REJECTED_LABEL, USAGE_SOURCE_HEADERS, Usage, UsageWindow, account_headroom,
    build_usage_result, build_usage_result_in, relevant_windows,
};

pub const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";
pub const OAUTH_EXPIRY_BUFFER_MS: i64 = 5 * 60 * 1000;
pub const OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
/// User-Agent on every request: haryx identifies as itself (upstream sends `claude-swap/1.0`). Like upstream's,
/// it is a non-first-party User-Agent, which the usage endpoint puts under a request budget (see
/// `usage_store`); the polling numbers in `poll_policy` were measured against that class, not a specific string.
/// The OAuth client id and beta header stay Claude Code's, since the stored tokens belong to that client.
pub const USER_AGENT: &str = concat!("haryx/", env!("CARGO_PKG_VERSION"));

/// What `claude setup-token` grants (and what `add-token` stores). Such a token can run inference
/// but cannot read the usage endpoint.
pub const SETUP_TOKEN_SCOPES: &[&str] = &["user:inference"];

/// The header probe: a 1-output-token Haiku request whose response headers carry the account's
/// unified 5h/7d utilization. The undated id names the current Haiku generation; once a model is
/// gone the probe fails with http-404 until the id is updated.
pub const USAGE_PROBE_URL: &str = "https://api.anthropic.com/v1/messages";
pub const USAGE_PROBE_MODEL: &str = "claude-haiku-5-5";
pub const USAGE_PROBE_API_VERSION: &str = "2023-06-01";
/// OAuth inference requests are only accepted with Claude Code's system prompt.
pub const USAGE_PROBE_SYSTEM: &str = "You are Claude Code, Anthropic's official CLI for Claude.";
/// Model id → the unified header prefix of the per-model weekly bucket its responses carry. A
/// Haiku probe never sees Fable's `7d_oi`, so with `usage.fableProbe` on, one more 1-token probe
/// per scoped model reads just that bucket. Sonnet and Opus probes carry no per-model bucket.
pub const USAGE_PROBE_SCOPED_MODELS: &[(&str, &str)] = &[("claude-fable-5-1", "7d_oi")];

/// How far ahead a stored login's expiry is worth showing. Claude Code's own notice fires inside
/// the last three days; a week leaves time to plan the re-logins of several accounts.
pub const LOGIN_DUE_WITHIN_S: f64 = 7.0 * 86400.0;

/// Test-only override: one base URL (e.g. `http://127.0.0.1:8123`) for every endpoint.
pub const API_BASE_ENV: &str = "HARYX_API_BASE";

/// The endpoints requests go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Api {
    pub token_url: String,
    pub usage_url: String,
    pub profile_url: String,
    pub messages_url: String,
}

impl Default for Api {
    fn default() -> Self {
        Api {
            token_url: OAUTH_TOKEN_URL.into(),
            usage_url: USAGE_URL.into(),
            profile_url: PROFILE_URL.into(),
            messages_url: USAGE_PROBE_URL.into(),
        }
    }
}

impl Api {
    /// Every endpoint under one base URL, keeping the real paths.
    pub fn with_base(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Api {
            token_url: format!("{base}/v1/oauth/token"),
            usage_url: format!("{base}/api/oauth/usage"),
            profile_url: format!("{base}/api/oauth/profile"),
            messages_url: format!("{base}/v1/messages"),
        }
    }

    /// The real endpoints, or `HARYX_API_BASE` when it is set and non-empty.
    pub fn from_env() -> Self {
        match std::env::var(API_BASE_ENV) {
            Ok(base) if !base.trim().is_empty() => Api::with_base(base.trim()),
            _ => Api::default(),
        }
    }
}

// --- Python value semantics used across the module ---

/// Python truthiness of a JSON value.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `isinstance(v, (int, float))` (which includes `bool`) as a float.
pub(crate) fn py_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// `float(v)`: numbers, bools and numeric strings.
pub(crate) fn py_float(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        other => py_num(other),
    }
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

fn now_s() -> f64 {
    format::ts_to_f64(jiff::Timestamp::now())
}

// --- Credential helpers ---

/// The OAuth access token from a credentials JSON string.
pub fn extract_access_token(credentials: &str) -> Option<String> {
    extract_oauth_data(credentials)?
        .get("accessToken")?
        .as_str()
        .map(str::to_string)
}

/// The `claudeAiOauth` payload of a credentials JSON string.
pub fn extract_oauth_data(credentials: &str) -> Option<serde_json::Map<String, Value>> {
    match serde_json::from_str::<Value>(credentials).ok()? {
        Value::Object(mut data) => match data.remove("claudeAiOauth") {
            Some(Value::Object(oauth)) => Some(oauth),
            _ => None,
        },
        _ => None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn nonempty_str<'a>(oauth: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    oauth
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Stable identity fingerprint for a stored credential.
///
/// Refresh-token hash when one exists (survives access-token rotation, so two generations of the
/// same OAuth lineage compare equal); full-content hash otherwise (API keys and setup-tokens never
/// rotate). `None` only for empty input — a caller comparing "did the credential change?" must
/// never get `None` for real bytes.
pub fn credential_fingerprint(credentials: &str) -> Option<String> {
    if credentials.is_empty() {
        return None;
    }
    let oauth = extract_oauth_data(credentials);
    match oauth.as_ref().and_then(|o| nonempty_str(o, "refreshToken")) {
        Some(token) => Some(format!("sha256:{}", sha256_hex(token.as_bytes()))),
        None => Some(format!(
            "sha256-full:{}",
            sha256_hex(credentials.as_bytes())
        )),
    }
}

/// Hash of the access token alone: the part that rotates within a lineage, so a refused token
/// and its replacement compare unequal.
pub fn access_token_fingerprint(credentials: &str) -> Option<String> {
    let oauth = extract_oauth_data(credentials)?;
    let token = nonempty_str(&oauth, "accessToken")?;
    Some(format!("sha256-at:{}", sha256_hex(token.as_bytes())))
}

/// Raw `refreshTokenExpiresAt` (epoch milliseconds), or `None` when absent/unreadable.
fn refresh_token_expires_at_ms(credentials: &str) -> Option<f64> {
    let oauth = extract_oauth_data(credentials)?;
    match oauth.get("refreshTokenExpiresAt")? {
        Value::Number(n) => n.as_f64().filter(|v| v.is_finite() && *v > 0.0),
        _ => None, // bools and strings are "unknown"
    }
}

/// When the stored *login* itself lapses, as ISO-8601 UTC (`2026-07-05T08:59:00Z`), or `None`.
///
/// Claude Code stores `refreshTokenExpiresAt` next to the access token's `expiresAt`. The access
/// token is renewed from the refresh token on its own; the refresh token is only replaced by a
/// fresh `/login`, so this is the date worth showing before the slot needs re-login. Logins issued
/// before Claude Code recorded the field carry nothing, which means "unknown", never "now".
pub fn login_expires_at_iso(credentials: &str) -> Option<String> {
    format::iso_utc_seconds_z(refresh_token_expires_at_ms(credentials)? / 1000.0)
}

/// Epoch-seconds twin of [`login_expires_at_iso`].
pub fn login_expires_at_epoch(credentials: &str) -> Option<f64> {
    refresh_token_expires_at_ms(credentials).map(|ms| ms / 1000.0)
}

/// Seconds until a stored login must be redone, when that is worth saying.
///
/// `None` when the stamp is unknown, further out than [`LOGIN_DUE_WITHIN_S`], or the slot is
/// quarantined (its usage sentinel already reads "re-login needed"). `0.0` once the stamp is past.
pub fn login_time_left(
    expires_at: Option<f64>,
    quarantined: bool,
    now: Option<f64>,
) -> Option<f64> {
    if quarantined {
        return None;
    }
    let remaining = expires_at? - now.unwrap_or_else(now_s);
    (remaining <= LOGIN_DUE_WITHIN_S).then_some(remaining.max(0.0))
}

/// `"re-login in 5d 12h"` while a stored login is due, `"re-login needed"` once its stamp is
/// past, `None` when there is nothing to say. `short` keeps the countdown's first unit.
pub fn format_login_expiry(
    expires_at: Option<f64>,
    quarantined: bool,
    now: Option<f64>,
    short: bool,
) -> Option<String> {
    let left = login_time_left(expires_at, quarantined, now)?;
    if left <= 0.0 {
        return Some("re-login needed".into());
    }
    let countdown = format_countdown(left);
    let shown = if short {
        countdown.split(' ').next().unwrap_or_default()
    } else {
        &countdown
    };
    Some(format!("re-login in {shown}"))
}

/// The login stamp as the local clock string the reset columns use.
pub fn login_expiry_clock(expires_at: f64, now: Option<f64>) -> String {
    let now = now
        .and_then(format::f64_to_ts)
        .unwrap_or_else(jiff::Timestamp::now);
    let at = format::f64_to_ts(expires_at).unwrap_or(now);
    reset_clock_string(at, now)
}

/// Whether an OAuth token is expired or about to expire (within [`OAUTH_EXPIRY_BUFFER_MS`]).
/// A non-numeric `expiresAt` is never expired.
pub fn is_oauth_token_expired(expires_at: Option<&Value>) -> bool {
    is_oauth_token_expired_at(expires_at, now_ms())
}

/// [`is_oauth_token_expired`] against an explicit clock.
pub fn is_oauth_token_expired_at(expires_at: Option<&Value>, now_ms: i64) -> bool {
    match expires_at.and_then(py_num) {
        Some(at) => (now_ms + OAUTH_EXPIRY_BUFFER_MS) as f64 >= at.trunc(),
        None => false,
    }
}

/// Whether an OAuth payload is an inference-only setup-token: no refresh token and exactly the
/// `user:inference` scope. The usage endpoint never answers such a token; its usage is read from
/// response headers instead (the header probe), or not at all.
pub fn is_setup_token(oauth: Option<&serde_json::Map<String, Value>>) -> bool {
    let Some(oauth) = oauth else { return false };
    if oauth.get("refreshToken").is_some_and(truthy) {
        return false;
    }
    let Some(Value::Array(scopes)) = oauth.get("scopes") else {
        return false;
    };
    let mut have: Vec<&str> = Vec::new();
    for s in scopes {
        match s.as_str() {
            Some(s) if !have.contains(&s) => have.push(s),
            Some(_) => {}
            None => return false,
        }
    }
    have.len() == SETUP_TOKEN_SCOPES.len() && SETUP_TOKEN_SCOPES.iter().all(|s| have.contains(s))
}

/// A short debug summary of stored OAuth token state. Never includes a token.
pub fn build_token_status(credentials: &str) -> Option<String> {
    build_token_status_at(
        credentials,
        jiff::Timestamp::now(),
        &jiff::tz::TimeZone::system(),
    )
}

/// [`build_token_status`] against an explicit clock and zone.
pub fn build_token_status_at(
    credentials: &str,
    now: jiff::Timestamp,
    tz: &jiff::tz::TimeZone,
) -> Option<String> {
    let oauth = extract_oauth_data(credentials).filter(|o| !o.is_empty())?;
    let refresh = if oauth.get("refreshToken").is_some_and(truthy) {
        "yes"
    } else {
        "no"
    };
    let Some(expires_at) = oauth.get("expiresAt").and_then(py_num) else {
        return Some(format!("oauth: unknown expiry, refresh token {refresh}"));
    };
    let state = if is_oauth_token_expired_at(oauth.get("expiresAt"), now.as_millisecond()) {
        "expired"
    } else {
        "fresh"
    };
    let iso = format::iso_utc(expires_at / 1000.0)?;
    let (countdown, clock) = format_reset_in(&iso, now, tz).ok()?;
    Some(format!(
        "oauth: {state}, refresh token {refresh}, expires {clock} in {countdown}"
    ))
}

#[cfg(test)]
pub(crate) mod tests;
