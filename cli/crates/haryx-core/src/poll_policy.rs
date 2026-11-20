//! Cadence policy for the `/api/oauth/usage` endpoint — every number in one place.
//! Port of `claude_swap/poll_policy.py`.
//!
//! The endpoint enforces a budget on non-first-party clients: a **~60-minute window of ~28-30
//! requests per identity × UA class** (measured upstream 2026-07-11). It is NOT a bucket with a
//! refill rate: capacity returns only as old requests age out of the trailing hour, so a burst
//! saturates the identity for up to a full hour and pausing does not restore headroom early.
//!
//! The identity depends on the org's 429 regime: under the fixed-deadline regime it is the
//! account/org (a freshly minted token was blocked 135 s after issue); under the saturated-edge
//! (`Retry-After: 0`) regime it is the access token. Plan for the account-scoped case — the
//! conservative one: re-authenticating cannot be relied on to clear a block, and two machines with
//! different tokens for one account may share one budget, which [`POST_429_BACKOFF_MULT`]
//! exists to converge.
//!
//! The horizon is bracketed to ~55-64 minutes and the exact edge algorithm is undocumented, so the
//! constants lean only on the robust parts: a sustained rate safely under the cap (an average of at
//! most ~1 request / 3 minutes, 20/hour vs the ~28-30 cap) and an ~hour recovery horizon. Health
//! invariant to watch in the logs: steady state shows zero http-429. A post-burst 429 does not
//! reliably clear at its stated horizon, which is why the usage store waits Retry-After plus a
//! margin.
//!
//! Plans computed here are persisted per account in the usage store (`nextPollAt` /
//! `pollIntervalS`) by whichever collector fetched, so every surface inherits the same cadence.
//! If a future probe revises the measured shape, adjust the constants in this module only.

use serde_json::Value;

use crate::lines::window_line;
use crate::oauth::{self, account_headroom, relevant_windows};

/// Freshness floor shared by every collector: an entry younger than this is served from the store
/// without any fetch, so the maximum sustained rate on one token is 1/SERVE_TTL_S.
pub const SERVE_TTL_S: f64 = 180.0;

/// Normal cadence floor — movement can halve an interval down to this, never below.
pub const MIN_INTERVAL_S: f64 = 180.0;

/// Urgent mode: the ACTIVE account, with some window that ROSE this poll to within
/// [`ESCALATION_MARGIN_PCT`] of its own switch line. Judged per window rather than on the binding
/// max (a 5h window closing on its line under a flat, higher weekly window is exactly the burn a
/// max-only rule is blind to). Bounded: either the line is crossed (the engine switches away) or
/// the rise stops — worst case ≈ 15 polls per episode, inside the rolling-hour cap.
pub const URGENT_INTERVAL_S: f64 = 60.0;

/// Decay ceilings for an account whose usage is not moving: the active account stays reasonably
/// fresh, an idle alternate drifts out to ten minutes.
pub const ACTIVE_MAX_INTERVAL_S: f64 = 300.0;
pub const CANDIDATE_DEFAULT_INTERVAL_S: f64 = 300.0;
pub const CANDIDATE_MAX_INTERVAL_S: f64 = 600.0;

/// Exhaustion is stable enough to poll slowly, but not to stop until a reported reset: quota
/// grants and provider-side corrections can make an account usable earlier, and decision-grade
/// status must not age into "unavailable". Six requests/hour stays below the budget.
pub const EXHAUSTED_INTERVAL_S: f64 = 600.0;

/// An account whose binding pct moved at least this much between polls is being consumed
/// somewhere → tighten; an unmoved one backs off toward its ceiling. Urgent mode reads the same
/// delta per window, rising only.
pub const MOVEMENT_DELTA_PCT: f64 = 1.0;

/// ±fraction applied to each scheduled interval so independent processes drift apart instead of
/// fetching in lockstep.
pub const JITTER_FRAC: f64 = 0.1;

/// Reaction to a 429 with `Retry-After: 0` (the saturated-window edge): probe at most every 5
/// minutes (≤12/hour) so aging-out — up to ~30/hour — outpaces the probing.
pub const EDGE_BACKOFF_S: f64 = 300.0;
/// While any 429 was seen on the token within [`RECENT_429_WINDOW_S`], floor the cadence here so
/// freed capacity accumulates instead of being re-spent. The window matches the saturation horizon.
pub const POST_429_MIN_INTERVAL_S: f64 = 360.0;
pub const RECENT_429_WINDOW_S: f64 = 3600.0;

/// AIMD on a contended budget shared across every machine polling the account: while 429s recur,
/// each successful poll multiplies the interval toward [`POST_429_MAX_INTERVAL_S`] — wider than the
/// candidate ceiling so several machines can each back off far enough that their combined rate
/// fits. Movement with no recent 429 decays it back down.
pub const POST_429_BACKOFF_MULT: f64 = 1.5;
pub const POST_429_MAX_INTERVAL_S: f64 = 1800.0;

/// The engine escalates to a full candidate refresh when any of the active account's windows is
/// within this margin of its own switch line; urgent-mode cadence keys on the same band.
pub const ESCALATION_MARGIN_PCT: f64 = 15.0;

/// Never schedule a poll later than a known window reset (+ slack).
pub const RESET_SLACK_S: f64 = 60.0;

/// The header probe's per-model request stops once that model's weekly bucket is spent: until it
/// resets another request can only be refused or billed to usage credits, which costs money the
/// user did not agree to spend. A spent response naming neither a reset nor a Retry-After is asked
/// about again after `SCOPED_PROBE_RETRY_S`; no deadline further than a weekly period is honoured.
pub const SCOPED_PROBE_RETRY_S: f64 = 3600.0;
pub const SCOPED_PROBE_MAX_PAUSE_S: f64 = 7.0 * 86400.0;

/// Utilization of the binding (worst) relevant window, or `None`.
pub fn binding_pct(usage: Option<&Value>, models: &[String]) -> Option<f64> {
    account_headroom(usage, models).map(|h| 100.0 - h)
}

/// Epoch when the last of the ≥100% relevant windows resets (account usable again).
pub fn limiting_reset_ts(usage: Option<&Value>, models: &[String]) -> Option<f64> {
    relevant_windows(usage, models)
        .iter()
        .filter(|w| w.pct >= 100.0)
        .filter_map(|w| parse_reset_ts(w.resets_at.as_deref()))
        .reduce(f64::max)
}

/// Epoch of the next relevant-window reset ahead of `now`, any utilization.
pub fn earliest_future_reset_ts(usage: Option<&Value>, now: f64, models: &[String]) -> Option<f64> {
    relevant_windows(usage, models)
        .iter()
        .filter_map(|w| parse_reset_ts(w.resets_at.as_deref()))
        .filter(|ts| *ts > now)
        .reduce(f64::min)
}

/// Epoch seconds of an ISO reset string (`Z` or an offset; a naive one is local time), or `None`
/// when missing, empty or unparseable.
pub fn parse_reset_ts(resets_at: Option<&str>) -> Option<f64> {
    let s = resets_at.filter(|s| !s.is_empty())?;
    oauth::parse_iso_epoch(s)
}

/// Epoch before which a per-model bucket just read as spent is not probed again.
///
/// The server's Retry-After when the refusal carried a positive one, else the bucket's reset,
/// else [`SCOPED_PROBE_RETRY_S`] from now. Never past the bucket's reset and never more than
/// [`SCOPED_PROBE_MAX_PAUSE_S`] away.
pub fn scoped_probe_after(now: f64, resets_at: Option<&str>, retry_after_s: Option<f64>) -> f64 {
    let reset_ts = parse_reset_ts(resets_at);
    let until = match (retry_after_s.filter(|r| *r > 0.0), reset_ts) {
        (Some(wait), _) => now + wait,
        (None, Some(reset)) => reset,
        (None, None) => now + SCOPED_PROBE_RETRY_S,
    };
    let until = reset_ts.map_or(until, |reset| until.min(reset));
    until.min(now + SCOPED_PROBE_MAX_PAUSE_S)
}

/// Whether a stored per-model row is still kept in place of a new probe: true while its
/// `probe_after` lies ahead and its own reset has not passed. A deadline further than
/// [`SCOPED_PROBE_MAX_PAUSE_S`] away is not one this module writes (an imported reading can carry
/// anything), so it pauses nothing.
pub fn scoped_probe_paused(row: &Value, now: f64) -> bool {
    let Some(until) = parse_reset_ts(row.get("probe_after").and_then(Value::as_str)) else {
        return false;
    };
    if !(now < until && until <= now + SCOPED_PROBE_MAX_PAUSE_S) {
        return false;
    }
    parse_reset_ts(row.get("resets_at").and_then(Value::as_str)).is_none_or(|reset| now < reset)
}

/// Inputs of [`plan_after_fetch`] for an account just fetched successfully.
#[derive(Debug, Clone, Copy)]
pub struct PlanInput<'a> {
    pub prev_interval_s: Option<f64>,
    pub prev_usage: Option<&'a Value>,
    pub new_usage: Option<&'a Value>,
    pub is_active: bool,
    pub threshold: f64,
    pub models: &'a [String],
    pub recent_429: bool,
    pub now: f64,
    pub weekly_threshold: Option<f64>,
}

/// The interval before jitter and reset capping.
fn plan_interval(p: &PlanInput<'_>) -> f64 {
    let default = if p.is_active {
        MIN_INTERVAL_S
    } else {
        CANDIDATE_DEFAULT_INTERVAL_S
    };
    let ceiling = if p.is_active {
        ACTIVE_MAX_INTERVAL_S
    } else {
        CANDIDATE_MAX_INTERVAL_S
    };
    // Python's `prev_interval_s or default`: 0 counts as unset.
    let base = p.prev_interval_s.filter(|v| *v != 0.0).unwrap_or(default);
    let prev_pct = binding_pct(p.prev_usage, p.models);
    let new_pct = binding_pct(p.new_usage, p.models);
    let mut interval = match (prev_pct, new_pct) {
        (Some(prev), Some(new)) if (new - prev).abs() >= MOVEMENT_DELTA_PCT => {
            MIN_INTERVAL_S.max(base / 2.0)
        }
        // Floored so a sub-floor base (urgent mode's 60s) snaps straight back to the normal
        // cadence once movement stops.
        (Some(_), Some(_)) => ceiling.min(MIN_INTERVAL_S.max(base * 1.5)),
        _ => default,
    };
    // Per window, by label, rising only: the window that matters may not be the max, and a window
    // that just dropped into the band is a rollover, not a burn.
    let prev_windows = relevant_windows(p.prev_usage, p.models);
    let rising_in_band = relevant_windows(p.new_usage, p.models).iter().any(|w| {
        let Some(prev) = prev_windows.iter().rev().find(|pw| pw.label == w.label) else {
            return false;
        };
        let line = window_line(&w.label, p.threshold, p.weekly_threshold);
        w.pct - prev.pct >= MOVEMENT_DELTA_PCT && w.pct >= line - ESCALATION_MARGIN_PCT
    });
    if p.is_active && rising_in_band && !p.recent_429 {
        interval = URGENT_INTERVAL_S;
    }
    if p.recent_429 {
        // AIMD: grow multiplicatively from the last interval toward the wider 429 ceiling;
        // floored at POST_429_MIN_INTERVAL_S for the first 429.
        let increased = (base * POST_429_BACKOFF_MULT).max(POST_429_MIN_INTERVAL_S);
        interval = POST_429_MAX_INTERVAL_S.min(interval.max(increased));
    }
    interval
}

/// `(next_poll_at, interval_s)` for an account just fetched successfully.
///
/// Movement (binding pct changed ≥ [`MOVEMENT_DELTA_PCT`]) halves the interval, floored at
/// [`MIN_INTERVAL_S`] — or drops to [`URGENT_INTERVAL_S`] when one of the active account's windows
/// rose that much into the escalation band under its own line. No movement backs off ×1.5 toward
/// the ceiling; unknown utilization uses the default. A recent 429 floors the cadence at
/// [`POST_429_MIN_INTERVAL_S`] (and suppresses urgent mode). The scheduled time gets
/// [`JITTER_FRAC`] noise and is never later than the next window reset (+ [`RESET_SLACK_S`]). An
/// at-limit account keeps a bounded slow poll instead of sleeping until that reset.
///
/// `rng` returns a uniform value in `[0, 1)`.
pub fn plan_after_fetch(p: &PlanInput<'_>, rng: &mut dyn FnMut() -> f64) -> (f64, f64) {
    let mut interval = plan_interval(p);
    let headroom = account_headroom(p.new_usage, p.models);
    let exhausted = headroom.is_some_and(|h| h <= 0.0);
    if exhausted {
        // Keep probing exhausted accounts; preserve a wider post-429 interval.
        interval = interval.max(EXHAUSTED_INTERVAL_S);
    }
    let mut next_poll = p.now + interval * (1.0 + JITTER_FRAC * (2.0 * rng() - 1.0));
    if exhausted {
        if let Some(reset) = limiting_reset_ts(p.new_usage, p.models).filter(|r| *r > p.now) {
            next_poll = next_poll.min(reset + RESET_SLACK_S);
        }
    } else if let Some(reset) = earliest_future_reset_ts(p.new_usage, p.now, p.models) {
        next_poll = next_poll.min(reset + RESET_SLACK_S);
    }
    (next_poll, interval)
}

/// [`plan_after_fetch`] with the process RNG.
pub fn plan_after_fetch_random(p: &PlanInput<'_>) -> (f64, f64) {
    plan_after_fetch(p, &mut || rand::random::<f64>())
}

#[cfg(test)]
mod tests;
