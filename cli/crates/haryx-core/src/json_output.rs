//! Port of `claude_swap/json_output.py`: the schema-v1 shapes of `--json` output.
//!
//! `list`, `status` and `switch` agree on field names (camelCase) and on how the internal usage map is
//! projected to JSON. Callers build payloads here; the CLI does the single serialisation. The
//! contract is `upstream/claude-swap/docs/json-output.md` and the fixtures in `packages/contracts/`.

use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::error::Error;
use crate::models::AccountPolicy;
use crate::oauth::{self, USAGE_SOURCE_HEADERS};
use crate::poll_policy::parse_reset_ts;
use crate::usage_store::Decision;
use crate::{pace, subscription};

/// Bump only on a breaking change to any payload shape. Scripts key off this.
pub const SCHEMA_VERSION: u32 = 1;

// Sentinel entries the collector yields in place of a usage map.
pub const USAGE_NO_CREDENTIALS: &str = "no credentials";
pub const USAGE_TOKEN_EXPIRED: &str = "token expired";
/// API-key accounts have no subscription quota.
pub const USAGE_API_KEY: &str = "api key";
/// The active account's macOS Keychain was unreadable with no plaintext fallback.
pub const USAGE_KEYCHAIN_UNAVAILABLE: &str = "keychain unavailable";
/// The stored refresh-token lineage is dead; only a re-login fixes it.
pub const USAGE_RELOGIN_REQUIRED: &str = "re-login needed";
/// The live credential was proven to belong to another account; a switch repairs the drift.
pub const USAGE_FOREIGN_CREDENTIAL: &str = "foreign credential";

type Object = Map<String, Value>;

/// `2026-11-21T10:00:00Z` (upstream `_timestamp`: seconds precision, `Z` suffix).
pub fn timestamp(epoch_s: f64) -> Value {
    let whole = epoch_s.floor();
    match Timestamp::from_second(whole as i64) {
        Ok(ts) => Value::String(ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
        Err(_) => Value::Null,
    }
}

/// Python's `round(x, 1)` for the values these payloads carry.
fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn window_to_json(entry: &Value) -> Object {
    let mut out = Object::new();
    out.insert(
        "pct".into(),
        entry.get("pct").cloned().unwrap_or(Value::Null),
    );
    if let Some(resets_at) = entry.get("resets_at") {
        out.insert("resetsAt".into(), resets_at.clone());
    }
    if let Some((countdown, clock)) = oauth::fresh_reset_strings(entry) {
        out.insert("countdown".into(), countdown.into());
        out.insert("clock".into(), clock.into());
    }
    out
}

/// Weekly-window pace fields (JSON-only; never on the 5h window).
fn pace_fields(entry: &Value, fetched_at: Option<f64>) -> Object {
    let mut out = Object::new();
    let Some(fetched_at) = fetched_at else {
        return out;
    };
    let Some(result) = pace::compute_pace(Some(entry), Some(fetched_at)) else {
        return out;
    };
    out.insert("expectedPct".into(), json!(round1(result.expected_pct)));
    out.insert("aheadOfPace".into(), json!(result.ahead));
    if let Some(eta) = pace::projected_exhaustion_ts(&result, fetched_at) {
        out.insert("projectedExhaustionAt".into(), timestamp(eta));
    }
    if let Some(will_last) = pace::will_last_to_reset(&result) {
        out.insert("willLastToReset".into(), json!(will_last));
    }
    out
}

fn weekly_window_to_json(entry: &Value, fetched_at: Option<f64>) -> Object {
    let mut out = window_to_json(entry);
    out.extend(pace_fields(entry, fetched_at));
    out
}

fn scoped_window_to_json(entry: &Value, fetched_at: Option<f64>) -> Object {
    let mut out = weekly_window_to_json(entry, fetched_at);
    out.insert(
        "name".into(),
        entry.get("name").cloned().unwrap_or(Value::Null),
    );
    if let Some(probe_after) = entry.get("probe_after") {
        out.insert("probeAfter".into(), probe_after.clone());
    }
    out
}

fn spend_to_json(spend: &Value) -> Object {
    let mut out = Object::new();
    for key in ["used", "limit", "pct", "currency"] {
        out.insert(key.into(), spend.get(key).cloned().unwrap_or(Value::Null));
    }
    if let Some(resets_at) = spend.get("resets_at") {
        out.insert("resetsAt".into(), resets_at.clone());
    }
    if let Some((countdown, clock)) = oauth::fresh_reset_strings(spend) {
        out.insert("countdown".into(), countdown.into());
        out.insert("clock".into(), clock.into());
    }
    out
}

/// The internal usage map's camelCase projection. `fetched_at` adds pace fields to weekly windows.
pub fn usage_to_json(usage: &Value, fetched_at: Option<f64>) -> Value {
    let mut out = Object::new();
    if let Some(w) = usage.get("five_hour") {
        out.insert("fiveHour".into(), Value::Object(window_to_json(w)));
    }
    if let Some(w) = usage.get("seven_day") {
        out.insert(
            "sevenDay".into(),
            Value::Object(weekly_window_to_json(w, fetched_at)),
        );
    }
    if let Some(spend) = usage.get("spend") {
        out.insert("spend".into(), Value::Object(spend_to_json(spend)));
    }
    if let Some(scoped) = usage.get("scoped").and_then(Value::as_array) {
        let windows = scoped
            .iter()
            .map(|w| Value::Object(scoped_window_to_json(w, fetched_at)))
            .collect();
        out.insert("scoped".into(), Value::Array(windows));
    }
    if usage.get("source").and_then(Value::as_str) == Some(USAGE_SOURCE_HEADERS) {
        out.insert("source".into(), USAGE_SOURCE_HEADERS.into());
    }
    if let Some(rejected) = usage.get("rejected").filter(|r| r.is_object()) {
        out.insert(
            "rejected".into(),
            json!({
                "retryAfterS": rejected.get("retry_after_s").cloned().unwrap_or(Value::Null),
                "resetsAt": rejected.get("resets_at").cloned().unwrap_or(Value::Null),
            }),
        );
    }
    Value::Object(out)
}

fn is_number(value: Option<&Value>) -> bool {
    value.and_then(Value::as_f64).is_some_and(f64::is_finite)
}

fn invalid(msg: impl Into<String>) -> Error {
    Error::Validation(msg.into())
}

fn window_from_json(window: &Value, label: &str, nullable: bool) -> Result<Object, Error> {
    let Some(obj) = window.as_object() else {
        return Err(invalid(format!("{label} must be an object")));
    };
    let pct = obj.get("pct");
    let pct_value = if nullable && pct.is_some_and(Value::is_null) {
        Value::Null
    } else if is_number(pct) && pct.and_then(Value::as_f64).is_some_and(|p| p >= 0.0) {
        json!(pct.and_then(Value::as_f64).unwrap_or_default())
    } else {
        let suffix = if nullable { " or null" } else { "" };
        return Err(invalid(format!(
            "{label}.pct must be a non-negative number{suffix}"
        )));
    };
    let mut out = Object::new();
    out.insert("pct".into(), pct_value);
    match obj.get("resetsAt") {
        None | Some(Value::Null) => {}
        Some(Value::String(resets_at)) => {
            let (countdown, clock) = oauth::format_reset(resets_at).map_err(|_| {
                invalid(format!(
                    "{label}.resetsAt is not an ISO-8601 time: '{resets_at}'"
                ))
            })?;
            out.insert("countdown".into(), countdown.into());
            out.insert("clock".into(), clock.into());
            out.insert("resets_at".into(), resets_at.clone().into());
        }
        Some(_) => {
            return Err(invalid(format!(
                "{label}.resetsAt must be an ISO-8601 string"
            )));
        }
    }
    Ok(out)
}

fn spend_from_json(spend: &Value) -> Result<Object, Error> {
    let mut out = window_from_json(spend, "spend", false)?;
    for key in ["used", "limit"] {
        if !is_number(spend.get(key)) {
            return Err(invalid(format!("spend.{key} must be a number")));
        }
        out.insert(key.into(), json!(spend[key].as_f64().unwrap_or_default()));
    }
    let Some(currency) = spend.get("currency").and_then(Value::as_str) else {
        return Err(invalid("spend.currency must be a string"));
    };
    out.insert("currency".into(), currency.into());
    Ok(out)
}

fn scoped_from_json(scoped: &Value) -> Result<Vec<Value>, Error> {
    let Some(list) = scoped.as_array() else {
        return Err(invalid("scoped must be a list"));
    };
    list.iter()
        .enumerate()
        .map(|(i, window)| {
            let label = format!("scoped[{i}]");
            let entry = window_from_json(window, &label, false)?;
            let name = window
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| invalid(format!("{label}.name must be a non-empty string")))?;
            let mut row = Object::new();
            row.insert("name".into(), name.into());
            row.extend(entry);
            match window.get("probeAfter") {
                None | Some(Value::Null) => {}
                Some(Value::String(p)) if parse_reset_ts(Some(p)).is_some() => {
                    row.insert("probe_after".into(), p.clone().into());
                }
                Some(_) => {
                    return Err(invalid(format!(
                        "{label}.probeAfter must be an ISO-8601 time"
                    )));
                }
            }
            Ok(Value::Object(row))
        })
        .collect()
}

fn rejected_from_json(rejected: &Value) -> Result<Value, Error> {
    if !rejected.is_object() {
        return Err(invalid("rejected must be an object"));
    }
    let retry_after = rejected.get("retryAfterS").cloned().unwrap_or(Value::Null);
    if !retry_after.is_null() && !is_number(Some(&retry_after)) {
        return Err(invalid("rejected.retryAfterS must be a number or null"));
    }
    let resets_at = rejected.get("resetsAt").cloned().unwrap_or(Value::Null);
    if !resets_at.is_null() && !resets_at.is_string() {
        return Err(invalid(
            "rejected.resetsAt must be an ISO-8601 string or null",
        ));
    }
    Ok(json!({"retry_after_s": retry_after, "resets_at": resets_at}))
}

/// A `usage` object from `list --json` back into the internal map (inverse of [`usage_to_json`] for what
/// the API measured). Errors on anything malformed, so an importer can refuse a document up front.
pub fn usage_from_json(usage: &Value) -> Result<Value, Error> {
    let Some(obj) = usage.as_object() else {
        return Err(invalid("usage must be an object"));
    };
    let mut out = Object::new();
    if let Some(w) = obj.get("fiveHour") {
        out.insert(
            "five_hour".into(),
            Value::Object(window_from_json(w, "fiveHour", true)?),
        );
    }
    if let Some(w) = obj.get("sevenDay") {
        out.insert(
            "seven_day".into(),
            Value::Object(window_from_json(w, "sevenDay", true)?),
        );
    }
    if let Some(spend) = obj.get("spend") {
        out.insert("spend".into(), Value::Object(spend_from_json(spend)?));
    }
    if let Some(scoped) = obj.get("scoped") {
        out.insert("scoped".into(), Value::Array(scoped_from_json(scoped)?));
    }
    if let Some(rejected) = obj.get("rejected") {
        out.insert("rejected".into(), rejected_from_json(rejected)?);
    }
    if out.is_empty() {
        return Err(invalid("usage carries no windows"));
    }
    if obj.get("source").and_then(Value::as_str) == Some(USAGE_SOURCE_HEADERS) {
        for (key, label) in [("five_hour", "fiveHour"), ("seven_day", "sevenDay")] {
            if out.get(key).is_some_and(|w| w["pct"].is_null()) {
                return Err(invalid(format!(
                    "{label}.pct cannot be null in a headers reading"
                )));
            }
        }
        out.insert("source".into(), USAGE_SOURCE_HEADERS.into());
    }
    Ok(Value::Object(out))
}

/// `(usageStatus, usage|null)` for a collected entry: a reading, a sentinel, or nothing (fetch failed).
pub fn usage_fields(entry: Option<Decision<'_>>, fetched_at: Option<f64>) -> (&'static str, Value) {
    match entry {
        Some(Decision::Usage(usage)) => ("ok", usage_to_json(usage, fetched_at)),
        Some(Decision::Sentinel(USAGE_TOKEN_EXPIRED)) => ("token_expired", Value::Null),
        Some(Decision::Sentinel(USAGE_API_KEY)) => ("api_key", Value::Null),
        Some(Decision::Sentinel(USAGE_KEYCHAIN_UNAVAILABLE)) => {
            ("keychain_unavailable", Value::Null)
        }
        Some(Decision::Sentinel(USAGE_RELOGIN_REQUIRED)) => ("relogin_required", Value::Null),
        Some(Decision::Sentinel(USAGE_FOREIGN_CREDENTIAL)) => ("foreign_credential", Value::Null),
        Some(Decision::Sentinel(_)) => ("no_credentials", Value::Null),
        None => ("unavailable", Value::Null),
    }
}

/// A minimal account reference, used for switch `from`/`to`.
pub fn account_ref(number: Option<i64>, email: &str) -> Value {
    json!({"number": number, "email": email})
}

/// `usageFetchedAt`/`usageAgeSeconds`: how old the served measurement is.
pub fn usage_freshness_fields(fetched_at: Option<f64>, age_s: Option<f64>) -> Object {
    let mut out = Object::new();
    let Some(fetched_at) = fetched_at else {
        return out;
    };
    out.insert("usageFetchedAt".into(), timestamp(fetched_at));
    if let Some(age) = age_s {
        out.insert("usageAgeSeconds".into(), json!(round1(age)));
    }
    out
}

/// `usageError`/`usageRetryAt`, only for an `unavailable` row with nothing else to explain it.
pub fn usage_failure_fields(
    status: &str,
    last_error: Option<&str>,
    backoff_until: Option<f64>,
) -> Object {
    let mut out = Object::new();
    let Some(err) = last_error.filter(|e| !e.is_empty() && status == "unavailable") else {
        return out;
    };
    out.insert("usageError".into(), err.into());
    if let Some(until) = backoff_until {
        out.insert("usageRetryAt".into(), timestamp(until));
    }
    out
}

/// Display-grade last-good usage, separate from decision-grade `usage`.
pub fn last_good_usage_fields(
    usage: Option<&Value>,
    fetched_at: Option<f64>,
    age_s: Option<f64>,
) -> Object {
    let mut out = Object::new();
    let (Some(usage), Some(fetched)) = (usage.filter(|u| u.is_object()), fetched_at) else {
        return out;
    };
    let freshness = usage_freshness_fields(Some(fetched), age_s);
    out.insert("lastGoodUsage".into(), usage_to_json(usage, Some(fetched)));
    out.insert(
        "lastGoodFetchedAt".into(),
        freshness["usageFetchedAt"].clone(),
    );
    if let Some(age) = freshness.get("usageAgeSeconds") {
        out.insert("lastGoodAgeSeconds".into(), age.clone());
    }
    out
}

/// JSON for a stored plan record (`{"plan": {...}, "fetchedAt": epoch}`).
pub fn subscription_plan_fields(record: &Value) -> Value {
    let plan = record.get("plan").and_then(Value::as_object);
    let mut out = Object::new();
    out.insert(
        "label".into(),
        subscription::plan_label(plan).map_or(Value::Null, Value::String),
    );
    out.insert(
        "tier".into(),
        plan.and_then(|p| p.get("tier"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    for key in ["organizationType", "billingType", "subscriptionStatus"] {
        if let Some(v) = plan.and_then(|p| p.get(key)) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(name) = plan
        .and_then(|p| p.get("planDisplayName"))
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    {
        out.insert("planDisplayName".into(), name.into());
    }
    out.insert("expired".into(), json!(subscription::is_expired(plan)));
    let fetched = record.get("fetchedAt").and_then(Value::as_f64);
    out.insert("fetchedAt".into(), fetched.map_or(Value::Null, timestamp));
    Value::Object(out)
}

/// Everything about one account a `list` row shows.
#[derive(Debug, Default)]
pub struct AccountRowInput<'a> {
    pub number: i64,
    pub email: &'a str,
    pub org_name: &'a str,
    pub org_uuid: &'a str,
    pub active: bool,
    pub usage_entry: Option<Decision<'a>>,
    pub usage_fetched_at: Option<f64>,
    pub usage_age_s: Option<f64>,
    pub last_good_usage: Option<&'a Value>,
    pub last_error: Option<&'a str>,
    /// The live backoff only; a lapsed one is the caller's to withhold.
    pub backoff_until: Option<f64>,
    pub alias: &'a str,
    /// The account's group as stored, "" when it has none.
    pub group: &'a str,
    pub disabled: bool,
    pub policy: AccountPolicy,
    pub login_expires_at: Option<&'a str>,
    pub subscription_plan: Option<&'a Value>,
}

/// A full account row for `list --json`. Optional keys are added only when they carry something, so a
/// fleet that never uses a feature emits rows identical to before it existed.
pub fn account_row(input: &AccountRowInput<'_>) -> Value {
    let (status, usage) = usage_fields(input.usage_entry, input.usage_fetched_at);
    let mut row = Object::new();
    row.insert("number".into(), json!(input.number));
    row.insert("email".into(), input.email.into());
    row.insert("organizationName".into(), input.org_name.into());
    row.insert("organizationUuid".into(), input.org_uuid.into());
    row.insert("isOrganization".into(), json!(!input.org_uuid.is_empty()));
    row.insert("active".into(), json!(input.active));
    row.insert("usageStatus".into(), status.into());
    let has_usage = !usage.is_null();
    row.insert("usage".into(), usage);
    if !input.alias.is_empty() {
        row.insert("alias".into(), input.alias.into());
    }
    if !input.group.is_empty() {
        row.insert("group".into(), input.group.into());
    }
    if input.disabled {
        row.insert("disabled".into(), json!(true));
    }
    if let Some(threshold) = input.policy.threshold {
        row.insert("threshold".into(), json!(threshold));
    }
    if let Some(expires) = input.login_expires_at.filter(|e| !e.is_empty()) {
        row.insert("loginExpiresAt".into(), expires.into());
    }
    if let Some(plan) = input.subscription_plan.filter(|p| !p.is_null()) {
        row.insert("plan".into(), subscription_plan_fields(plan));
    }
    if has_usage {
        row.extend(usage_freshness_fields(
            input.usage_fetched_at,
            input.usage_age_s,
        ));
    } else {
        row.extend(last_good_usage_fields(
            input.last_good_usage,
            input.usage_fetched_at,
            input.usage_age_s,
        ));
        row.extend(usage_failure_fields(
            status,
            input.last_error,
            input.backoff_until,
        ));
    }
    Value::Object(row)
}

/// The upstream exception class name of an error, as `type(exc).__name__` reports it.
pub fn exception_name(err: &Error) -> &'static str {
    match err {
        Error::General(_) | Error::Io(_) | Error::Json(_) => "ClaudeSwitchError",
        Error::Credential(_) => "CredentialError",
        Error::CredentialRead(_) => "CredentialReadError",
        Error::CredentialWrite(_) => "CredentialWriteError",
        Error::Config(_) => "ConfigError",
        Error::Switch(_) => "SwitchError",
        Error::Session(_) => "SessionError",
        Error::Lock(_) => "LockError",
        Error::ClaudeCodeLockTimeout(_) => "ClaudeCodeLockTimeout",
        Error::AccountNotFound(_) => "AccountNotFoundError",
        Error::Validation(_) => "ValidationError",
        Error::Transfer(_) => "TransferError",
        Error::Migration(_) => "MigrationError",
    }
}

/// The structured error payload emitted on a handled error.
pub fn error_envelope(err: &Error) -> Value {
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "error": {"type": exception_name(err), "message": err.to_string()},
    })
}

#[cfg(test)]
mod tests;
