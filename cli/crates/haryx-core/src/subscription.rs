//! Subscription plan per account, for display only (upstream #350). Port of
//! `claude_swap/subscription.py`.
//!
//! The plan comes from `GET /api/oauth/profile` — the same endpoint
//! [`crate::oauth::fetch_oauth_profile`] calls to resolve identity; five `organization` fields are
//! kept here. Measured shapes, one account each:
//!
//! | state               | status   | rate_limit_tier        | billing_type        |
//! |---------------------|----------|------------------------|---------------------|
//! | active Max          | active   | default_claude_max_20x | stripe_subscription |
//! | cancelled, in period| active   | default_claude_max_20x | stripe_subscription |
//! | lapsed              | canceled | default_claude_ai      | none                |
//!
//! An Ultra account carries `default_claude_max_20x`, organization type `claude_max` and
//! `plan_display_name` "Ultra", so the tier alone does not name the plan. An Enterprise seat
//! carries `default_claude_zero` and no plan name, so its organization type claims the label.
//!
//! `subscription_status` does not move at cancellation, only at lapse, so renewal is not shown.
//! Expired is `canceled` together with the free tier — both, so neither field alone marks it.
//!
//! Paced on its own clock ([`PLAN_TTL_S`], six hours) and stored in its own file, never through the
//! usage store: a plan refresh riding on the rate-limited usage endpoint would freeze an old label
//! behind exactly the accounts whose usage calls are failing.
//!
//! A refresh only ever uses an access token that is already valid ([`usable_access_token`]); this
//! module never refreshes a token itself. A response is only believed once its account and
//! organization UUIDs match the slot ([`matches_slot`]). A failed or mismatched read sets
//! `retryAt` beside the record and leaves `fetchedAt` alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::error::Result;
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::oauth::{self, Api, FetchError, Headers, USER_AGENT};
use crate::settings::atomic_write_json;
use crate::usage_store::{Clock, system_clock};

pub const PLAN_FILENAME: &str = "subscription_plans.json";

/// Plans change on a billing cadence, not a usage one: one profile call per account four times a
/// day keeps a lapse visible the same day.
pub const PLAN_TTL_S: f64 = 6.0 * 3600.0;
/// After a failed read: at least this long, longer if the server says so. A Retry-After of 0 is
/// real on this API, so it cannot be the only floor.
pub const FAILURE_RETRY_S: f64 = 15.0 * 60.0;
/// Ceiling on any wait, and the bound past which a stored `retryAt` is clock skew.
pub const MAX_RETRY_S: f64 = 24.0 * 3600.0;
/// Default timeout of [`fetch_plan`].
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

pub const FREE_TIER: &str = "default_claude_ai";

/// Labels of observed tiers only: an unknown tier falls back to the raw string rather than a
/// guessed label — a wrong "Pro" is worse than an honest "default_claude_pro".
const TIER_LABELS: &[(&str, &str)] = &[
    ("default_claude_ai", "Free"),
    ("default_claude_max_5x", "Max 5x"),
    ("default_claude_max_20x", "Max 20x"),
];

/// A Team/Enterprise seat is not a personal subscription whatever rate tier it carries, and a
/// paying Pro subscription reports the default bucket (`default_claude_ai`, which a lapsed account
/// also falls back to), so the organization type wins where it is known.
const ORG_TYPE_LABELS: &[(&str, &str)] = &[
    ("claude_team", "Team"),
    ("claude_enterprise", "Enterprise"),
    ("claude_pro", "Pro"),
];

const KEPT_FIELDS: &[(&str, &str)] = &[
    ("organization_type", "organizationType"),
    ("billing_type", "billingType"),
    ("subscription_status", "subscriptionStatus"),
    ("plan_display_name", "planDisplayName"),
];

/// A stored plan (`tier` plus the kept camelCase fields).
pub type Plan = Map<String, Value>;

/// One profile read. `plan` is `None` on any failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanFetch {
    pub plan: Option<Plan>,
    pub account_uuid: String,
    pub organization_uuid: String,
    pub retry_after_s: Option<f64>,
}

/// Read the plan fields of `/api/oauth/profile` (the real endpoint, or `HARYX_API_BASE`).
///
/// Must not be called while a credential or config lock is held.
pub fn fetch_plan(access_token: &str, timeout: Duration) -> PlanFetch {
    fetch_plan_from(&Api::from_env().profile_url, access_token, timeout)
}

/// [`fetch_plan`] against an explicit profile URL.
pub fn fetch_plan_from(url: &str, access_token: &str, timeout: Duration) -> PlanFetch {
    match get_json(url, access_token, timeout) {
        Ok(data) => parse_plan(&data),
        Err(err) => {
            // Display-only; never let it surface.
            log::debug!("Plan fetch failed: {err}");
            PlanFetch {
                retry_after_s: err.classify().1,
                ..PlanFetch::default()
            }
        }
    }
}

/// The plan of a profile response. Requires a non-empty `rate_limit_tier`, the field every label
/// derives from; a response without one is no answer rather than a plan with nothing in it.
pub fn parse_plan(data: &Value) -> PlanFetch {
    let Some(organization) = data.get("organization").and_then(Value::as_object) else {
        return PlanFetch::default();
    };
    let Some(tier) = nonempty(organization.get("rate_limit_tier")) else {
        return PlanFetch::default();
    };
    let mut plan = Plan::new();
    plan.insert("tier".into(), json!(tier));
    for (source, key) in KEPT_FIELDS {
        if let Some(value) = nonempty(organization.get(*source)) {
            plan.insert((*key).into(), json!(value));
        }
    }
    // Always present (null if unsent): `is_due` reads a missing key as an old record.
    plan.entry("planDisplayName").or_insert(Value::Null);
    let account_uuid = data
        .get("account")
        .and_then(|a| a.get("uuid"))
        .and_then(Value::as_str);
    PlanFetch {
        plan: Some(plan),
        account_uuid: account_uuid.unwrap_or_default().to_owned(),
        organization_uuid: organization
            .get("uuid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        retry_after_s: None,
    }
}

fn nonempty(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `GET url` with the bearer token, as JSON. Errors map onto [`FetchError`] so its classification
/// (and the Retry-After) applies.
// TODO(oauth): reuse `oauth::http::get` once it is exported to the crate.
fn get_json(url: &str, access_token: &str, timeout: Duration) -> Result<Value, FetchError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .header("Authorization", &format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(transport_error)?;
    let status = resp.status().as_u16();
    let headers = Headers::new(resp.headers().iter().map(|(k, v)| {
        (
            k.as_str().to_owned(),
            String::from_utf8_lossy(v.as_bytes()).into_owned(),
        )
    }));
    let body = resp.body_mut().read_to_vec().map_err(transport_error)?;
    if !(200..300).contains(&status) {
        return Err(FetchError::Http {
            status,
            headers,
            body,
        });
    }
    serde_json::from_slice(&body).map_err(|e| FetchError::Json(e.to_string()))
}

fn transport_error(err: ureq::Error) -> FetchError {
    match err {
        ureq::Error::Timeout(_) => FetchError::Timeout,
        other => FetchError::Network(other.to_string()),
    }
}

/// Whether a read provably belongs to the slot it was made for.
///
/// Every UUID the slot records must match, and at least one must exist: a slot recording neither
/// cannot confirm anything, so it shows no plan rather than one that may be another account's.
pub fn matches_slot(fetched: &PlanFetch, account_uuid: &str, org_uuid: &str) -> bool {
    let expected: Vec<(&str, &str)> = [
        (account_uuid, fetched.account_uuid.as_str()),
        (org_uuid, fetched.organization_uuid.as_str()),
    ]
    .into_iter()
    .filter(|(recorded, _)| !recorded.is_empty())
    .collect();
    !expected.is_empty() && expected.iter().all(|(recorded, seen)| recorded == seen)
}

/// When a failed read may be repeated.
pub fn next_retry_at(now: f64, retry_after_s: Option<f64>) -> f64 {
    let wait = FAILURE_RETRY_S.max(retry_after_s.unwrap_or(0.0));
    now + wait.min(MAX_RETRY_S)
}

fn lookup(table: &[(&str, &'static str)], key: Option<&str>) -> Option<&'static str> {
    let key = key?;
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Human label: organization type where known, else the server's plan name, else the tier's label,
/// else the raw tier.
pub fn plan_label(plan: Option<&Plan>) -> Option<String> {
    let plan = plan.filter(|p| !p.is_empty())?;
    let str_of = |key: &str| plan.get(key).and_then(Value::as_str);
    if let Some(label) = lookup(ORG_TYPE_LABELS, str_of("organizationType")) {
        return Some(label.to_owned());
    }
    let tier = str_of("tier");
    let label = lookup(TIER_LABELS, tier).or(tier);
    // The server's name wins unless the table names the same plan more precisely ("Max" vs
    // "Max 5x"); Ultra shares the Max 20x tier string.
    match str_of("planDisplayName").filter(|n| !n.is_empty()) {
        Some(name) if !label.is_some_and(|l| l.starts_with(name)) => Some(name.to_owned()),
        _ => label.map(str::to_owned),
    }
}

/// A subscription that has lapsed: `canceled` AND back on the free tier.
pub fn is_expired(plan: Option<&Plan>) -> bool {
    plan.is_some_and(|p| {
        !p.is_empty()
            && p.get("subscriptionStatus").and_then(Value::as_str) == Some("canceled")
            && p.get("tier").and_then(Value::as_str) == Some(FREE_TIER)
    })
}

/// Store key. Identity, not slot number — `swap`/`move` renumber slots.
pub fn identity_key(email: &str, org_uuid: Option<&str>) -> String {
    format!("{email}\n{}", org_uuid.unwrap_or_default())
}

/// The access token in `credentials` if it is present and unexpired.
pub fn usable_access_token(credentials: Option<&str>) -> Option<String> {
    let blob = oauth::extract_oauth_data(credentials.filter(|c| !c.is_empty())?)?;
    let token = blob.get("accessToken").and_then(Value::as_str)?;
    if token.is_empty() || oauth::is_oauth_token_expired(blob.get("expiresAt")) {
        return None;
    }
    Some(token.to_owned())
}

/// A number that is not a bool (`isinstance(v, (int, float)) and not isinstance(v, bool)`).
fn real_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

/// Whether a stored record has a plan with a string tier and a numeric `fetchedAt`.
pub fn valid_record(record: &Value) -> bool {
    record
        .get("plan")
        .and_then(Value::as_object)
        .is_some_and(|plan| plan.get("tier").is_some_and(Value::is_string))
        && real_number(record.get("fetchedAt")).is_some()
}

/// [`is_due_within`] at [`PLAN_TTL_S`].
pub fn is_due(record: Option<&Value>, now: f64) -> bool {
    is_due_within(record, now, PLAN_TTL_S)
}

/// Whether a record needs refreshing. A future stamp is clock skew: refresh.
///
/// A live `retryAt` holds the read back whatever the age of the plan; one further out than any
/// wait this module sets is skew and is ignored.
pub fn is_due_within(record: Option<&Value>, now: f64, ttl: f64) -> bool {
    let Some(record) = record else {
        return true;
    };
    if let Some(retry_at) = real_number(record.get("retryAt")) {
        if now < retry_at && retry_at <= now + MAX_RETRY_S {
            return false;
        }
    }
    if !valid_record(record) {
        return true;
    }
    if record["plan"].get("planDisplayName").is_none() {
        return true; // stored before the name was kept
    }
    let fetched_at = real_number(record.get("fetchedAt")).unwrap_or(f64::NEG_INFINITY);
    fetched_at > now || now - fetched_at >= ttl
}

/// A stored stamp that may take part in ordering: a number no later than `now`. A stamp ahead of
/// the clock was written before the clock was set back; it says nothing about recency.
fn past_stamp(record: &Map<String, Value>, key: &str, now: f64) -> Option<f64> {
    real_number(record.get(key)).filter(|v| *v <= now)
}

/// One key of a save. The newer reading wins, whichever process lands last.
///
/// `update` is a success ([`success_update`]) or a failure ([`failure_update`]). Both are stamped
/// before their request went out, so a slow answer cannot pass for a newer one, and an older failure
/// cannot shorten the wait a newer one set. A stored stamp ahead of `now` is skew and never
/// outranks the update.
fn merge(existing: Option<&Value>, update: &Map<String, Value>, now: f64) -> Value {
    let current = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let current_value = Value::Object(current.clone());
    let fetched = valid_record(&current_value)
        .then(|| past_stamp(&current, "fetchedAt", now))
        .flatten();
    let attempted = past_stamp(&current, "attemptedAt", now);
    if update.contains_key("plan") {
        let update_at = real_number(update.get("fetchedAt")).unwrap_or(f64::NEG_INFINITY);
        if fetched.is_some_and(|f| f > update_at) {
            return current_value;
        }
        if let Some(attempted) = attempted.filter(|a| *a > update_at) {
            let mut merged = update.clone();
            merged.insert(
                "retryAt".into(),
                current.get("retryAt").cloned().unwrap_or(Value::Null),
            );
            merged.insert("attemptedAt".into(), json!(attempted));
            return Value::Object(merged);
        }
        return Value::Object(update.clone());
    }
    let update_at = real_number(update.get("attemptedAt")).unwrap_or(f64::NEG_INFINITY);
    if fetched.is_some_and(|f| f >= update_at) || attempted.is_some_and(|a| a > update_at) {
        return current_value;
    }
    let mut merged = current;
    merged.extend(update.clone());
    Value::Object(merged)
}

/// A successful read, stamped with the time its request went out.
pub fn success_update(plan: Plan, fetched_at: f64) -> Map<String, Value> {
    let mut update = Map::new();
    update.insert("plan".into(), Value::Object(plan));
    update.insert("fetchedAt".into(), json!(fetched_at));
    update
}

/// A failed or mismatched read: no read before `retry_at`.
pub fn failure_update(retry_at: f64, attempted_at: f64) -> Map<String, Value> {
    let mut update = Map::new();
    update.insert("retryAt".into(), json!(retry_at));
    update.insert("attemptedAt".into(), json!(attempted_at));
    update
}

/// `subscription_plans.json`: `{identity_key: record}`.
///
/// Several surfaces may refresh at once. `save` holds a lock across its read/merge/write, and only
/// that: the network calls happen before it, so the lock never waits on a slow response.
#[derive(Clone)]
pub struct PlanStore {
    pub path: PathBuf,
    clock: Clock,
}

impl std::fmt::Debug for PlanStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PlanStore {
    /// The store at `path`, on the system clock.
    pub fn new(path: &Path) -> Self {
        Self::with_clock(path, Arc::new(system_clock))
    }

    pub fn with_clock(path: &Path, clock: Clock) -> Self {
        PlanStore {
            path: path.to_path_buf(),
            clock,
        }
    }

    /// Every stored record; empty when missing or unreadable.
    pub fn load(&self) -> Map<String, Value> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|raw| match raw {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Merge `updates` (key → [`success_update`] / [`failure_update`]) into the file.
    pub fn save(&self, updates: &Map<String, Value>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut lock_name = self.path.file_name().unwrap_or_default().to_os_string();
        lock_name.push(".lock");
        let _lock = FileLock::lock(self.path.with_file_name(lock_name), DEFAULT_LOCK_TIMEOUT)?;
        // Read the clock under the lock: a record another writer landed while this one waited
        // carries a later stamp, and would be mistaken for skew against an earlier clock read.
        let now = (self.clock)();
        let mut merged = self.load();
        for (key, update) in updates {
            let Some(update) = update.as_object() else {
                continue;
            };
            let value = merge(merged.get(key), update, now);
            merged.insert(key.clone(), value);
        }
        atomic_write_json(&self.path, &Value::Object(merged))
    }
}

#[cfg(test)]
mod tests;
