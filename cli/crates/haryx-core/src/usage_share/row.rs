//! The shared `usage` row's values and their conversion to and from [`SharedState`].
//!
//! Times in the row are on the server's clock; [`Clock`] converts with the offset measured from
//! the server's `Date` header, so machines with different clocks agree on every deadline.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::poll_policy::LEDGER_WINDOW_S;
use crate::sync::merge::Values;
use crate::usage_store::{LOCAL_MACHINE, SharedState};

/// Local time minus server time, in seconds (0 when the server's clock is unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Clock {
    pub skew: f64,
}

impl Clock {
    /// `server_now` is a whole second read from a `Date` header, so the true server time lies
    /// somewhere in the second after it: its middle is the best guess.
    pub fn new(local_now: f64, server_now: Option<i64>) -> Self {
        Clock {
            skew: server_now.map_or(0.0, |server| local_now - (server as f64 + 0.5)),
        }
    }

    pub fn to_local(self, server: f64) -> f64 {
        server + self.skew
    }

    pub fn to_server(self, local: f64) -> f64 {
        local - self.skew
    }
}

fn number(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn time(values: &Values, key: &str, clock: Clock) -> Option<f64> {
    values
        .get(key)
        .and_then(Value::as_f64)
        .filter(|t| t.is_finite())
        .map(|t| clock.to_local(t))
}

fn text(values: &Values, key: &str) -> Option<String> {
    values
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The identity a row names (inside its encryption).
#[cfg(test)]
pub fn identity(values: &Values) -> Option<(String, String)> {
    let email = text(values, "email")?.to_lowercase();
    let org = values
        .get("org")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Some((email, org))
}

/// A row's values as state on this machine's clock.
pub fn read(values: &Values, clock: Clock) -> SharedState {
    let requests = values
        .get("requests")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    let at = pair.first()?.as_f64().filter(|t| t.is_finite())?;
                    let by = pair.get(1)?.as_str().filter(|s| !s.is_empty())?;
                    Some((clock.to_local(at), by.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let active_on = values
        .get("activeOn")
        .and_then(Value::as_object)
        .map(|marks| {
            marks
                .iter()
                .filter_map(|(by, until)| {
                    let until = until.as_f64().filter(|t| t.is_finite())?;
                    Some((by.clone(), clock.to_local(until)))
                })
                .collect()
        })
        .unwrap_or_default();
    SharedState {
        last_good: values.get("lastGood").filter(|v| v.is_object()).cloned(),
        fetched_at: time(values, "fetchedAt", clock),
        fetched_by: text(values, "fetchedBy"),
        next_poll_at: time(values, "nextPollAt", clock),
        poll_interval_s: values.get("pollIntervalS").and_then(Value::as_f64),
        backoff_until: time(values, "backoffUntil", clock),
        last_429_at: time(values, "last429At", clock),
        last_error: text(values, "lastError"),
        claim_until: time(values, "claimUntil", clock),
        claim_by: text(values, "claimBy"),
        requests,
        active_on,
    }
}

/// What this machine writes for one account. `me` replaces [`LOCAL_MACHINE`] in the ledger and
/// names a reading this machine took. Ledger entries and active marks past their window are
/// dropped.
pub fn write(
    identity: &(String, String),
    state: &SharedState,
    me: &str,
    clock: Clock,
    now: f64,
) -> Values {
    let server = |local: Option<f64>| local.map_or(Value::Null, |t| number(clock.to_server(t)));
    let requests: Vec<Value> = state
        .requests
        .iter()
        .filter(|(at, _)| *at > now - LEDGER_WINDOW_S)
        .map(|(at, by)| {
            let by = if by == LOCAL_MACHINE { me } else { by };
            json!([number(clock.to_server(*at)), by])
        })
        .collect();
    let active_on: Map<String, Value> = state
        .active_on
        .iter()
        .filter(|(_, until)| **until > now)
        .map(|(by, until)| (by.clone(), number(clock.to_server(*until))))
        .collect();
    let fetched_by = state.fetched_by.clone().unwrap_or_else(|| me.to_owned());
    BTreeMap::from([
        ("email".to_owned(), json!(identity.0)),
        ("org".to_owned(), json!(identity.1)),
        (
            "lastGood".to_owned(),
            state.last_good.clone().unwrap_or(Value::Null),
        ),
        ("fetchedAt".to_owned(), server(state.fetched_at)),
        ("fetchedBy".to_owned(), json!(fetched_by)),
        ("nextPollAt".to_owned(), server(state.next_poll_at)),
        (
            "pollIntervalS".to_owned(),
            state.poll_interval_s.map_or(Value::Null, number),
        ),
        ("backoffUntil".to_owned(), server(state.backoff_until)),
        ("last429At".to_owned(), server(state.last_429_at)),
        (
            "lastError".to_owned(),
            state.last_error.as_ref().map_or(Value::Null, |e| json!(e)),
        ),
        ("claimUntil".to_owned(), server(state.claim_until)),
        (
            "claimBy".to_owned(),
            state.claim_by.as_ref().map_or(Value::Null, |b| json!(b)),
        ),
        ("requests".to_owned(), Value::Array(requests)),
        ("activeOn".to_owned(), Value::Object(active_on)),
    ])
}
