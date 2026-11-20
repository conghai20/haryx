//! The read model ([`UsageEntry`]), the write input ([`FetchRecord`]) and the auto engine's
//! one-alternate-per-tick pick ([`due_candidate`]).

use std::collections::BTreeMap;

use serde_json::Value;

use super::policy::{live_claim, plan_oversleeps};
use super::{AUTH_DEAD_STRIKES, STALE_OK_S};
use crate::poll_policy::{RECENT_429_WINDOW_S, SERVE_TTL_S};

/// Outcome of one fetch attempt, as handed to [`super::UsageStore::record`].
///
/// Exactly one of three shapes:
/// - success: `error` and `sentinel` are `None` (`usage` may still be `None` when the response
///   carried no window data);
/// - failure: `error` set (with optional `retry_after_s`);
/// - sentinel: `sentinel` set — a derived state ("token expired" with an owner present, ...),
///   recorded as a no-op: sentinels are re-derived every pass and never persisted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchRecord {
    pub usage: Option<Value>,
    pub error: Option<String>,
    pub retry_after_s: Option<f64>,
    pub sentinel: Option<String>,
    /// Fingerprint of the credential whose refresh token was POSTed when a permanent auth error came
    /// back. Strikes bind to it: [`UsageEntry::token_dead`] holds only while the stored credential
    /// still fingerprints to the struck generation, so any credential-writing path heals the strike.
    /// `None` on non-auth outcomes (and from legacy writers — those strikes bind unconditionally).
    pub struck_fp: Option<String>,
    /// Hash of the access token a live session's read-only fetch was refused with. Rides on the
    /// sentinel and is the one thing a sentinel record persists; a success clears it.
    pub rejected_fp: Option<String>,
}

impl FetchRecord {
    /// A successful fetch (`usage` `None` when the response carried no window data).
    pub fn success(usage: Option<Value>) -> Self {
        FetchRecord {
            usage,
            ..Self::default()
        }
    }

    /// A failed fetch with its error kind and the server's Retry-After.
    pub fn failure(error: impl Into<String>, retry_after_s: Option<f64>) -> Self {
        FetchRecord {
            error: Some(error.into()),
            retry_after_s,
            ..Self::default()
        }
    }

    /// A derived sentinel state.
    pub fn sentinel(sentinel: impl Into<String>) -> Self {
        FetchRecord {
            sentinel: Some(sentinel.into()),
            ..Self::default()
        }
    }

    /// Bind a permanent-auth strike to this credential fingerprint.
    pub fn with_struck_fp(self, fp: Option<String>) -> Self {
        FetchRecord {
            struck_fp: fp,
            ..self
        }
    }

    /// Stamp the refused access token's hash (sentinel records).
    pub fn with_rejected_fp(self, fp: Option<String>) -> Self {
        FetchRecord {
            rejected_fp: fp,
            ..self
        }
    }
}

/// What a switch decision runs on: a sentinel, or a trusted reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision<'a> {
    Sentinel(&'a str),
    Usage(&'a Value),
}

/// Read model of one account's usage state at collect time.
///
/// `sentinel` is the collector's live overlay (never persisted); all other fields mirror the stored
/// row, except `age_s` (the age of `last_good`) and `trust_extended`, both computed at snapshot
/// time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageEntry {
    pub sentinel: Option<String>,
    /// The last good measurement (a JSON object), or `None`.
    pub last_good: Option<Value>,
    pub fetched_at: Option<f64>,
    pub age_s: Option<f64>,
    pub last_attempt_at: Option<f64>,
    pub consecutive_failures: i64,
    pub last_error: Option<String>,
    pub backoff_until: Option<f64>,
    pub next_poll_at: Option<f64>,
    pub poll_interval_s: Option<f64>,
    /// When this token last answered 429 (any Retry-After). Deliberately NOT cleared by a later
    /// success: the planner floors the cadence at `POST_429_MIN_INTERVAL_S` until
    /// `RECENT_429_WINDOW_S` has passed.
    pub last_429_at: Option<f64>,
    /// Consecutive permanent-auth failures (`invalid_grant`). At or above [`AUTH_DEAD_STRIKES`]
    /// the token is treated as dead: see [`UsageEntry::token_dead`].
    pub auth_dead_strikes: i64,
    /// Fingerprint of the generation the strikes condemned (absent on legacy rows → strikes bind
    /// unconditionally).
    pub struck_fingerprint: Option<String>,
    /// The refused access token a live session's fetch stamped; `None` once a fetch succeeds.
    pub rejected_fingerprint: Option<String>,
    /// Staleness past [`STALE_OK_S`] is still decision-trusted when it is *deliberate*: the server
    /// is refusing fresher data (failure state), the scheduler chose the cadence (within
    /// `nextPollAt`), a fetch is in flight, or another machine's reading is held. Capped at
    /// `TRUST_MAX_AGE_S`. Computed by `UsageStore::entries`.
    pub trust_extended: bool,
    pub claim_until: Option<f64>,
    /// Until when a reading adopted from another machine keeps every collector off this slot.
    pub held_until: Option<f64>,
}

impl UsageEntry {
    /// Whether the reading is within `ttl` (default [`SERVE_TTL_S`]).
    pub fn fresh(&self, now: f64) -> bool {
        self.fresh_within(now, SERVE_TTL_S)
    }

    pub fn fresh_within(&self, now: f64, ttl: f64) -> bool {
        self.fetched_at.is_some_and(|at| (now - at) <= ttl)
    }

    pub fn in_backoff(&self, now: f64) -> bool {
        self.backoff_until.is_some_and(|until| now < until)
    }

    /// Whether an adopted reading's hold still keeps collectors off.
    pub fn held(&self, now: f64) -> bool {
        self.held_until.is_some_and(|until| now < until)
    }

    /// Whether this token 429'd recently enough to keep the post-429 cadence.
    ///
    /// Recency is measured from when the 429's honored backoff *lifts*, not from the 429 itself:
    /// an hour-scale Retry-After is one long backoff, so measuring from the stamp would make the
    /// first post-block success see an already-elapsed window. The backoff anchor is used only
    /// while the live backoff is a 429 one (`lastError == "http-429"`), so an unrelated timeout on a
    /// token that 429'd long ago cannot re-arm it.
    pub fn recent_429(&self, now: f64) -> bool {
        let Some(stamp) = self.last_429_at else {
            return false;
        };
        let anchor = match self.backoff_until {
            Some(until) if self.last_error.as_deref() == Some("http-429") && until > stamp => until,
            _ => stamp,
        };
        now < anchor + RECENT_429_WINDOW_S
    }

    /// Whether another collector's bounded fetch lease is still live.
    pub fn claimed(&self, now: f64) -> bool {
        live_claim(self.claim_until, self.last_attempt_at, now)
    }

    /// [`UsageEntry::token_dead_with`] at the default threshold, with no stored fingerprint.
    pub fn token_dead(&self) -> bool {
        self.token_dead_with(AUTH_DEAD_STRIKES, None)
    }

    /// Whether the stored credential's refresh-token lineage is provably dead.
    ///
    /// True once `invalid_grant` has recurred `threshold` times without an intervening success.
    /// Strikes condemn the GENERATION that was POSTed, not the slot: when `stored_fp` (the
    /// currently stored credential's fingerprint) differs from the struck one, the credential was
    /// replaced since the verdict and the strike no longer applies. A row struck before
    /// fingerprints were recorded binds unconditionally.
    pub fn token_dead_with(&self, threshold: i64, stored_fp: Option<&str>) -> bool {
        if self.auth_dead_strikes < threshold {
            return false;
        }
        !matches!(
            (stored_fp, self.struck_fingerprint.as_deref()),
            (Some(stored), Some(struck)) if stored != struck
        )
    }

    /// The value switch decisions run on.
    ///
    /// Sentinel wins; else `last_good` while it is recent enough to trust (≤ [`STALE_OK_S`], or
    /// `trust_extended` for deliberate staleness); else `None` (unknown). Display code reads
    /// `last_good`/`age_s` directly instead — it may show older data, annotated with its age.
    pub fn decision_value(&self) -> Option<Decision<'_>> {
        if let Some(sentinel) = &self.sentinel {
            return Some(Decision::Sentinel(sentinel));
        }
        match (&self.last_good, self.age_s) {
            (Some(usage), Some(age)) if age <= STALE_OK_S || self.trust_extended => {
                Some(Decision::Usage(usage))
            }
            _ => None,
        }
    }
}

/// Whether a row carries an obsolete reset-parked plan.
///
/// Reset-parking releases stored a distant reset deadline while retaining the much shorter learned
/// interval. Detect that impossible shape structurally, independent of the current model selection,
/// so changing scoped models cannot leave an otherwise usable account parked until the old reset.
pub fn plan_oversleeps_interval(entry: &UsageEntry, now: f64) -> bool {
    plan_oversleeps(entry.next_poll_at, entry.poll_interval_s, now)
}

/// The due candidate least recently attempted or measured, or `None`.
///
/// Due = past its `nextPollAt` (or carrying an impossible parked plan), not in failure backoff,
/// not quarantined and not held for another machine's reading. Sentinel accounts have nothing to
/// fetch. Least recently touched wins — the later of the last attempt and the last measurement —
/// so a no-op sentinel pick rotates to the back instead of taking every tick; a never-touched row
/// keys 0 and a missing entry is most due.
pub fn due_candidate(
    candidates: &[String],
    entries: &BTreeMap<String, UsageEntry>,
    now: f64,
) -> Option<String> {
    let mut due: Vec<(f64, &String)> = candidates
        .iter()
        .filter_map(|num| match entries.get(num) {
            None => Some((0.0, num)),
            Some(entry) => is_due(entry, now).then(|| {
                let touched = entry
                    .fetched_at
                    .unwrap_or(0.0)
                    .max(entry.last_attempt_at.unwrap_or(0.0));
                (touched, num)
            }),
        })
        .collect();
    due.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    due.first().map(|(_, num)| (*num).clone())
}

fn is_due(entry: &UsageEntry, now: f64) -> bool {
    if entry.sentinel.is_some() || entry.token_dead() || entry.in_backoff(now) || entry.held(now) {
        return false;
    }
    let planned_ahead = entry.next_poll_at.is_some_and(|at| now < at);
    !planned_ahead || plan_oversleeps_interval(entry, now)
}

/// Overlay a derived sentinel state on a stored entry (read model only).
pub fn with_sentinel(entry: UsageEntry, sentinel: Option<&str>) -> UsageEntry {
    match sentinel {
        None => entry,
        Some(s) => UsageEntry {
            sentinel: Some(s.to_owned()),
            ..entry
        },
    }
}
