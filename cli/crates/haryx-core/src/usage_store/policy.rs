//! Pure decisions of the usage store: claim liveness, fetch eligibility, failure backoff and the
//! trust bounds of stale readings. Everything here reads a stored row or a reading and the clock;
//! nothing does I/O.

use serde_json::{Map, Value};

use super::{
    AUTH_DEAD_STRIKES, BACKOFF_BASE_S, BACKOFF_CAP_S, BACKOFF_MAX_SHIFT, LEGACY_CLAIM_TTL_S,
    RATE_LIMIT_TRUST_MAX_AGE_S, RETRY_AFTER_FLOOR_CAP_S, RETRY_AFTER_MARGIN_S, TRUST_MAX_AGE_S,
};
use crate::oauth::relevant_windows;
use crate::poll_policy::{
    EDGE_BACKOFF_S, EXHAUSTED_INTERVAL_S, JITTER_FRAC, RESET_SLACK_S, SERVE_TTL_S, parse_reset_ts,
};

/// `isinstance(v, (int, float))` as a float (Python's `bool` counts), else `None`.
pub(crate) fn num(value: Option<&Value>) -> Option<f64> {
    value.and_then(crate::oauth::py_num)
}

/// Python's `int(row.get(key) or 0)`. A value `int()` would reject reads as 0: collection never
/// raises.
pub(crate) fn int_or_zero(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .unwrap_or(0),
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Whether a collector's fetch lease on a row is still live.
///
/// `claim_until` (the fenced lease) wins when present; rows written by an older process fall back to
/// `lastAttemptAt` + [`LEGACY_CLAIM_TTL_S`]. The single source of truth for claim liveness —
/// `UsageEntry::claimed`, `entries()` and `row_eligible` must all agree, or concurrent collectors
/// double-fetch. (`record()` deliberately checks only the fenced form.)
pub(crate) fn live_claim(claim_until: Option<f64>, last_attempt_at: Option<f64>, now: f64) -> bool {
    match claim_until {
        Some(until) => now < until,
        None => last_attempt_at.is_some_and(|at| (now - at) < LEGACY_CLAIM_TTL_S),
    }
}

/// Whether a deadline cannot have come from the bounded planner.
pub(crate) fn plan_oversleeps(
    next_poll_at: Option<f64>,
    poll_interval_s: Option<f64>,
    now: f64,
) -> bool {
    let Some(next_poll_at) = next_poll_at else {
        return false;
    };
    // Python's `poll_interval_s or EXHAUSTED_INTERVAL_S`: 0 counts as unset.
    let interval = poll_interval_s
        .filter(|v| *v != 0.0)
        .unwrap_or(EXHAUSTED_INTERVAL_S)
        .max(EXHAUSTED_INTERVAL_S);
    let latest_normal_poll = now + interval * (1.0 + JITTER_FRAC) + RESET_SLACK_S;
    next_poll_at > latest_normal_poll
}

/// Epoch of the soonest relevant window to roll over, or `None` if unknown.
///
/// The soonest one is what matters: once it resets, usage there is zeroed and the whole snapshot is
/// obsolete, so a later window cannot rescue it. Windows carrying no `resets_at` contribute nothing
/// rather than a guess.
pub(crate) fn earliest_reset(last_good: Option<&Value>, models: &[String]) -> Option<f64> {
    relevant_windows(last_good, models)
        .iter()
        .filter_map(|w| parse_reset_ts(w.resets_at.as_deref()))
        .reduce(f64::min)
}

/// Whether 429-stale `last_good` is still trustworthy for decisions.
///
/// Usage rises monotonically within a window, so a rate-limited (frozen) reading is a valid lower
/// bound until its window resets — but only up to a client-side ceiling, so a far-future or
/// malformed `resets_at` can never grant unbounded trust:
///
/// `now < min(earliest relevant-window reset, now + (RATE_LIMIT_TRUST_MAX_AGE_S - age))`
///
/// The **earliest** reset, not the latest: once the soonest window rolls over the snapshot is
/// obsolete. A window without `resets_at` contributes no timestamp, so partial metadata can only
/// tighten the bound. `models` adds the per-model scoped windows that also gate the account.
pub(crate) fn rate_limited_trust_ok(
    last_good: Option<&Value>,
    age_s: Option<f64>,
    now: f64,
    models: &[String],
) -> bool {
    let Some(age_s) = age_s else {
        return false;
    };
    let ceiling = now + (RATE_LIMIT_TRUST_MAX_AGE_S - age_s);
    let bound = earliest_reset(last_good, models).map_or(ceiling, |soonest| soonest.min(ceiling));
    now < bound
}

fn rejected_of(usage: Option<&Value>) -> Option<&Map<String, Value>> {
    usage?.get("rejected")?.as_object()
}

/// A positive `retry_after_s` on a confirmed probe rejection.
fn positive_retry_after(rejected: &Map<String, Value>) -> Option<f64> {
    num(rejected.get("retry_after_s")).filter(|r| *r > 0.0)
}

/// When a reading from a refused setup-token header probe (`rejected` with a Retry-After) allows
/// the next request, or `None`. Stored as `backoffUntil`, which every fetch path's eligibility
/// already honors. Bounded by the same park cap as a 429's ask, so an absurd value cannot wedge
/// the row.
pub(crate) fn rejection_backoff_until(usage: Option<&Value>, now: f64) -> Option<f64> {
    let retry_after = positive_retry_after(rejected_of(usage)?)?;
    Some(now + retry_after.min(RETRY_AFTER_FLOOR_CAP_S))
}

/// Absolute end of the server's Retry-After on a confirmed probe rejection a reading carries,
/// bounded like [`rejection_backoff_until`], or `None` when unusable or already past.
///
/// For a reading taken on another machine the Retry-After was relative to that machine's fetch, so
/// only the absolute `rejected.resets_at` stamp carries over. Without a positive `retry_after_s`
/// that stamp is the quota reset the probe copied from the unified headers, not a deadline.
pub(crate) fn rejection_deadline(usage: Option<&Value>, now: f64) -> Option<f64> {
    let rejected = rejected_of(usage)?;
    positive_retry_after(rejected)?;
    let ts = parse_reset_ts(rejected.get("resets_at").and_then(Value::as_str))?;
    (ts > now).then(|| ts.min(now + RETRY_AFTER_FLOOR_CAP_S))
}

/// Seconds to stay in backoff after a failed fetch.
///
/// - No Retry-After: `30s · 2^(n-1)`, capped at [`BACKOFF_CAP_S`].
/// - `Retry-After: 0` on a 429 is the saturated-budget edge: wait at least
///   [`EDGE_BACKOFF_S`]. On a non-429 (e.g. a Cloudflare 503 "retry now") it is not that rule, so
///   the plain curve applies.
/// - An ask above [`BACKOFF_CAP_S`] on a 429 gets [`RETRY_AFTER_MARGIN_S`] on top: retrying ON
///   the deadline was measured to re-block for a fresh hour. A short ask was measured accurate and
///   takes no margin. The margin is 429-only: it was measured on usage-endpoint blocks, whose
///   stale data stays trusted up to [`RATE_LIMIT_TRUST_MAX_AGE_S`].
/// - PARK BOUND: every ask is capped by the ceiling its own arm's trust uses —
///   [`RETRY_AFTER_FLOOR_CAP_S`] on a 429, [`TRUST_MAX_AGE_S`] otherwise — so a pathological
///   header (`86400`, `inf`) cannot park a row for a day. It bounds the park, not the blind
///   window: `entries()` measures trust from the last success, the park from the failure.
/// - No trim against the server's deadline nor against our own trust: measured, both only add
///   requests inside a block that probing cannot shorten (see upstream's long comment).
///
/// Our own curve may still wait longer than the ask.
pub(crate) fn failure_backoff_s(
    consecutive_failures: i64,
    retry_after_s: Option<f64>,
    rate_limited: bool,
) -> f64 {
    // Exponent clamp: the count grows forever on a permanently failing account; the curve already
    // saturates at the cap by shift 5, so any clamp above that is behaviour-preserving.
    let shift = (consecutive_failures - 1).clamp(0, BACKOFF_MAX_SHIFT);
    // `shift` is in 0..=32 by the clamp, so the cast is lossless.
    let computed = (BACKOFF_BASE_S * 2f64.powi(shift as i32)).min(BACKOFF_CAP_S);
    let Some(ask) = retry_after_s else {
        return computed;
    };
    if ask == 0.0 {
        if !rate_limited {
            return computed;
        }
        // Saturated-budget edge: wait before probing again.
        // EDGE_BACKOFF_S (300) < BACKOFF_CAP_S (600), so the clamp bounds are ordered.
        return computed.clamp(EDGE_BACKOFF_S, BACKOFF_CAP_S);
    }
    let mut asked = ask;
    if ask > BACKOFF_CAP_S && rate_limited {
        asked = ask + RETRY_AFTER_MARGIN_S;
    }
    let park_cap = if rate_limited {
        RETRY_AFTER_FLOOR_CAP_S
    } else {
        TRUST_MAX_AGE_S
    };
    asked.min(park_cap).max(computed)
}

/// Fetch eligibility of a stored row, evaluated under the write lock (see
/// [`super::UsageStore::reserve`] for the two caller modes).
pub(crate) fn row_eligible(
    row: &Map<String, Value>,
    now: f64,
    respect_plans: bool,
    repair_overslept: bool,
) -> bool {
    if int_or_zero(row.get("authDeadStrikes")) >= AUTH_DEAD_STRIKES {
        return false;
    }
    if num(row.get("backoffUntil")).is_some_and(|until| now < until) {
        return false;
    }
    if num(row.get("heldUntil")).is_some_and(|until| now < until) {
        return false;
    }
    // The hourly cap binds whatever the plan says: every machine's fetches are in the ledger.
    if super::peer::ledger_full(row, now) {
        return false;
    }
    if live_claim(
        num(row.get("claimUntil")),
        num(row.get("lastAttemptAt")),
        now,
    ) {
        return false;
    }
    let stale = num(row.get("fetchedAt")).is_none_or(|at| (now - at) > SERVE_TTL_S);
    let next_poll_at = num(row.get("nextPollAt"));
    let poll_due = next_poll_at.is_some_and(|at| now >= at);
    let overslept =
        repair_overslept && plan_oversleeps(next_poll_at, num(row.get("pollIntervalS")), now);
    if respect_plans {
        return stale && (poll_due || next_poll_at.is_none() || overslept);
    }
    if repair_overslept {
        return poll_due || (stale && (next_poll_at.is_none() || overslept));
    }
    poll_due || stale
}
