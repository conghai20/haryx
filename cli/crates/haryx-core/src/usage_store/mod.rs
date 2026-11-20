//! Per-account usage table: last-known-good measurements + fetch/backoff state. Port of
//! `claude_swap/usage_store.py`.
//!
//! Lives in `cache/usage.json` (`schemaVersion: 2`; a version-less legacy file or a future schema
//! is treated as empty). One failed round trip no longer blanks every account: a failure updates
//! the error/backoff fields and never touches the last-good measurement (stale-on-error). The table
//! is shared by on-demand surfaces (`list`/`status`, dashboards) and the auto engine's scheduled
//! polling, so each learns from the other's fetches.
//!
//! The store persists only *measurements* (`lastGood`) and *fetch state* (failures, backoff, poll
//! schedule). Sentinel states ("api key", "token expired", ...) are derived fresh by the collector
//! on every pass and overlaid on the read model ([`UsageEntry::sentinel`]) — never written, so a
//! stale sentinel can't outlive the condition that produced it.
//!
//! Locking protocol (never holds the lock across network I/O):
//! (a) lock → read, decide/claim the fetch set (stamp `claimUntil`) → unlock ([`UsageStore::reserve`]);
//! (b) fetch with no lock held;
//! (c) lock → re-read, merge outcomes, clear the claim, write → unlock ([`UsageStore::record`]).
//! The bounded lease lets concurrent collectors skip accounts another process is still fetching;
//! recording the outcome releases it and a crashed claimer ages out. The collector records per
//! *batch*, so the lease TTL is sized for the whole batch, not one request.
//!
//! The cadence constants come from [`crate::poll_policy`]; the constants below are the store's own
//! trust, lease and backoff bounds, as upstream keeps them here.

mod entry;
mod policy;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};

pub use crate::poll_policy::SERVE_TTL_S;
pub use entry::{
    Decision, FetchRecord, UsageEntry, due_candidate, plan_oversleeps_interval, with_sentinel,
};

use crate::error::Result;
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::settings::atomic_write_json;
use policy::{
    earliest_reset, failure_backoff_s, int_or_zero, live_claim, num, rate_limited_trust_ok,
    rejection_backoff_until, rejection_deadline, row_eligible,
};

/// Version stamped in `usage.json`; anything else reads as empty.
pub const SCHEMA_VERSION: i64 = 2;
/// File name under the cache directory.
pub const USAGE_FILENAME: &str = "usage.json";
const LOCK_FILENAME: &str = ".usage.lock";

/// Trusted for switch decisions; older → headroom unknown (unless trust is extended).
pub const STALE_OK_S: f64 = 300.0;
/// In-flight claim window: covers the bounded refresh + usage path, the inventory's stagger and
/// queueing, so another surface cannot reclaim a request still in flight.
pub const CLAIM_TTL_S: f64 = 90.0;
/// Claim window of rows written by an older collector (no `claimUntil`).
pub const LEGACY_CLAIM_TTL_S: f64 = 10.0;
/// Ceiling on deliberate staleness: a forever-failing account must eventually read as unknown, even
/// past a longer Retry-After — trust must never be server-controlled and unbounded.
pub const TRUST_MAX_AGE_S: f64 = 3600.0;
/// Fallback ceiling for 429-stale data. A usage-endpoint 429 throttles polling without moving the
/// account's windows, and usage only rises within a window, so a 429-stale reading is a lower bound
/// trusted until its earliest window reset — never past this age.
pub const RATE_LIMIT_TRUST_MAX_AGE_S: f64 = 7200.0;
/// Failure backoff without a Retry-After: `30s · 2^(n-1)`, capped.
pub const BACKOFF_BASE_S: f64 = 30.0;
pub const BACKOFF_CAP_S: f64 = 600.0;
/// Exponent clamp, so a saturated failure counter never overflows the curve.
pub const BACKOFF_MAX_SHIFT: i64 = 32;
/// Added to an hour-scale 429 Retry-After: measured, 20 of 35 lapses retried within 900s of their
/// own deadline re-blocked for a fresh hour (band edge +887s).
pub const RETRY_AFTER_MARGIN_S: f64 = 900.0;
/// Park cap of a 429 ask: the measured 3600s block plus the margin.
pub const RETRY_AFTER_FLOOR_CAP_S: f64 = 4500.0;
/// Strikes of permanent-auth failure after which a token is quarantined as dead.
pub const AUTH_DEAD_STRIKES: i64 = 1;
/// Errors that prove the stored credential is permanently unusable; only these advance the strike
/// count.
pub const PERMANENT_AUTH_ERRORS: &[&str] = &["invalid_grant", "no_refresh_token"];

/// `(email, organizationUuid)` — the identity a slot number currently maps to.
pub type Identity = (String, String);
/// Slot number → identity.
pub type Identities = BTreeMap<String, Identity>;
/// A scheduler plan: `(nextPollAt, pollIntervalS)`.
pub type PollPlan = (Option<f64>, Option<f64>);
/// The store's clock, in epoch seconds.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

type Row = Map<String, Value>;
type Rows = Map<String, Value>;

/// The system clock in epoch seconds.
pub fn system_clock() -> f64 {
    jiff::Timestamp::now().as_microsecond() as f64 / 1e6
}

/// A JSON number, or `null` for a non-finite value (JSON has no infinity).
fn number(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn opt_number(value: Option<f64>) -> Value {
    value.map_or(Value::Null, number)
}

fn new_claim_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn matches(row: Option<&Value>, identity: &Identity) -> bool {
    row.and_then(Value::as_object).is_some_and(|row| {
        row.get("email").and_then(Value::as_str) == Some(identity.0.as_str())
            && row.get("organizationUuid").map_or(Some(""), Value::as_str)
                == Some(identity.1.as_str())
    })
}

fn fresh_row(identity: &Identity) -> Value {
    json!({"email": identity.0, "organizationUuid": identity.1})
}

/// The row for `num`, replaced with a fresh one first when it belongs to another account.
fn matching_row<'a>(rows: &'a mut Rows, num: &str, identity: &Identity) -> &'a mut Row {
    if !matches(rows.get(num), identity) {
        rows.insert(num.to_owned(), fresh_row(identity));
    }
    row_mut(rows, num)
}

fn row_mut<'a>(rows: &'a mut Rows, num: &str) -> &'a mut Row {
    let value = rows.entry(num.to_owned()).or_insert_with(|| json!({}));
    if !value.is_object() {
        *value = json!({});
    }
    match value {
        Value::Object(row) => row,
        // Made an object just above.
        _ => unreachable!("row is an object"),
    }
}

fn str_field(row: &Row, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The `cache/usage.json` table. All writes go read-modify-write under `cache/.usage.lock`; reads
/// are lock-free (writes are atomic replaces).
///
/// Every method takes the caller's current `identities` map and only ever touches rows for those
/// slots: a row whose stored identity differs is invisible to reads and replaced on write, so slot
/// reuse never serves the previous account's usage. Rows for slots outside the map are left alone.
/// A slot named in an argument but missing from `identities` is skipped.
#[derive(Clone)]
pub struct UsageStore {
    pub path: PathBuf,
    lock_path: PathBuf,
    clock: Clock,
}

impl std::fmt::Debug for UsageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl UsageStore {
    /// The store in `cache_dir` (`<store root>/cache`), on the system clock.
    pub fn new(cache_dir: &Path) -> Self {
        Self::with_clock(cache_dir, Arc::new(system_clock))
    }

    pub fn with_clock(cache_dir: &Path, clock: Clock) -> Self {
        UsageStore {
            path: cache_dir.join(USAGE_FILENAME),
            lock_path: cache_dir.join(LOCK_FILENAME),
            clock,
        }
    }

    /// The store's clock, for stores that must agree with it.
    pub fn clock(&self) -> Clock {
        Arc::clone(&self.clock)
    }

    /// The store's current time.
    pub fn now(&self) -> f64 {
        (self.clock)()
    }

    // -- raw I/O ---------------------------------------------------------------------------------

    fn lock(&self) -> Result<FileLock> {
        FileLock::lock(&self.lock_path, DEFAULT_LOCK_TIMEOUT)
    }

    fn read_rows(&self) -> Rows {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Rows::new();
        };
        let Ok(Value::Object(mut raw)) = serde_json::from_str::<Value>(&text) else {
            return Rows::new();
        };
        // Python's `!= 2`, under which `2.0` (and nothing else) also matches.
        if num(raw.get("schemaVersion")) != Some(SCHEMA_VERSION as f64) {
            return Rows::new(); // legacy snapshot or future schema: start empty
        }
        match raw.remove("accounts") {
            Some(Value::Object(rows)) => rows,
            _ => Rows::new(),
        }
    }

    fn write_rows(&self, rows: Rows) -> Result<()> {
        atomic_write_json(
            &self.path,
            &json!({"schemaVersion": SCHEMA_VERSION, "accounts": Value::Object(rows)}),
        )
    }

    /// Read-modify-write rows for `nums` under the lock. A row whose stored identity mismatches is
    /// replaced with a fresh one first.
    fn mutate<'a>(
        &self,
        identities: &Identities,
        nums: impl IntoIterator<Item = &'a String>,
        mut mutator: impl FnMut(&str, &mut Row),
    ) -> Result<()> {
        let _lock = self.lock()?;
        let mut rows = self.read_rows();
        for num in nums {
            let Some(identity) = identities.get(num) else {
                continue;
            };
            mutator(num, matching_row(&mut rows, num, identity));
        }
        self.write_rows(rows)
    }

    // -- read model ------------------------------------------------------------------------------

    /// Identity-guarded snapshot for the given slots (an empty entry when the row is missing or
    /// belongs to a different account).
    ///
    /// `models` are the configured scoped-window model names; they let the 429-stale trust bound
    /// also honor per-model window resets, matching the scheduler's window view.
    pub fn entries(
        &self,
        identities: &Identities,
        models: &[String],
    ) -> BTreeMap<String, UsageEntry> {
        let now = self.now();
        let rows = self.read_rows();
        identities
            .iter()
            .map(|(num, identity)| {
                let entry = match rows.get(num).and_then(Value::as_object) {
                    Some(row) if matches(rows.get(num), identity) => entry_of(row, now, models),
                    _ => UsageEntry::default(),
                };
                (num.clone(), entry)
            })
            .collect()
    }

    // -- writes ----------------------------------------------------------------------------------

    /// Lease the slots about to be fetched, returning their fencing ids.
    pub fn claim(
        &self,
        nums: &[String],
        identities: &Identities,
    ) -> Result<BTreeMap<String, String>> {
        if nums.is_empty() {
            return Ok(BTreeMap::new());
        }
        let now = self.now();
        let claims: BTreeMap<String, String> = nums
            .iter()
            .filter(|num| identities.contains_key(*num))
            .map(|num| (num.clone(), new_claim_id()))
            .collect();
        self.mutate(identities, claims.keys(), |num, row| {
            stamp_claim(row, now, &claims[num]);
        })?;
        Ok(claims)
    }

    /// Atomically win the right to fetch: re-check eligibility and stamp a bounded lease in one
    /// locked pass, returning slot → fencing id.
    ///
    /// Deciding eligibility on a lock-free [`UsageStore::entries`] read and then claiming separately
    /// lets two collectors both pass the check and both fetch; the re-check under the lock closes
    /// that window. Eligibility: not quarantined (dead token), not in failure backoff, not held for
    /// another machine's reading ([`UsageStore::adopt`]), not claimed within [`CLAIM_TTL_S`], and
    /// then by caller mode —
    ///
    /// - `respect_plans = true` (on-demand callers: list/status/switch, dashboards): the entry must
    ///   be stale (older than [`SERVE_TTL_S`]) *and* poll-due (past `nextPollAt`, or no plan yet).
    ///   With `repair_overslept`, a structurally obsolete reset-parked plan is also due.
    /// - `respect_plans = false` (the auto engine's deliberate schedule): poll-due *or* stale — a due
    ///   entry may be re-fetched inside the serve TTL (how the urgent cadence beats the TTL), and an
    ///   escalation refresh may fetch a not-yet-due candidate. With `repair_overslept`, due plans
    ///   and stale impossible plans win, but valid future plans do not.
    pub fn reserve(
        &self,
        nums: &[String],
        identities: &Identities,
        respect_plans: bool,
        repair_overslept: bool,
    ) -> Result<BTreeMap<String, String>> {
        let mut won = BTreeMap::new();
        if nums.is_empty() {
            return Ok(won);
        }
        let now = self.now();
        let _lock = self.lock()?;
        let mut rows = self.read_rows();
        for num in nums {
            let Some(identity) = identities.get(num) else {
                continue;
            };
            let known = matches(rows.get(num), identity);
            let row = matching_row(&mut rows, num, identity);
            if known && !row_eligible(row, now, respect_plans, repair_overslept) {
                continue;
            }
            let claim_id = new_claim_id();
            stamp_claim(row, now, &claim_id);
            won.insert(num.clone(), claim_id);
        }
        if !won.is_empty() {
            self.write_rows(rows)?;
        }
        Ok(won)
    }

    /// Merge outcomes fenced by the leases that produced them; returns the accepted slots.
    ///
    /// A late writer whose lease or slot identity was replaced is ignored without touching the
    /// newer row. Success and failure are mutually exclusive writers: success resets the failure
    /// fields, failure never touches `lastGood`/`fetchedAt`. A supplied success plan commits in the
    /// same transaction as its measurement. Sentinel records clear only the claim and are otherwise
    /// never persisted, save the refused-credential stamp one may carry. Unfenced callers (no
    /// `claims`) defer to a live lease but never to an expired one.
    pub fn record(
        &self,
        outcomes: &BTreeMap<String, FetchRecord>,
        identities: &Identities,
        claims: Option<&BTreeMap<String, String>>,
        plans: Option<&BTreeMap<String, PollPlan>>,
    ) -> Result<BTreeSet<String>> {
        let mut accepted = BTreeSet::new();
        if outcomes.is_empty() {
            return Ok(accepted);
        }
        let now = self.now();
        let _lock = self.lock()?;
        let mut rows = self.read_rows();
        for (num, rec) in outcomes {
            let Some(identity) = identities.get(num) else {
                continue;
            };
            if !writer_accepted(rows.get(num), identity, claims.map(|c| c.get(num)), now) {
                continue;
            }
            let row = matching_row(&mut rows, num, identity);
            apply_outcome(row, rec, plans.and_then(|p| p.get(num)), now);
            accepted.insert(num.clone());
        }
        if !accepted.is_empty() {
            self.write_rows(rows)?;
        }
        Ok(accepted)
    }

    /// Merge measurements another machine took for the same accounts; returns the slots whose
    /// `lastGood` was replaced.
    ///
    /// `readings` maps slot → `(usage, age_s)`: how old the measurement was when handed over, so
    /// the two machines' clocks never have to agree (`fetchedAt` becomes `now - age_s`). A reading
    /// replaces `lastGood` only when newer than the stored one. Fetch state is left alone, except a
    /// confirmed probe rejection's deadline, which extends (never shortens) `backoffUntil`.
    ///
    /// `hold_s > 0` stamps `heldUntil`: no collector fetches the slot before then, and its
    /// last-good stays decision-trusted meanwhile. The hold never runs past the stored reading's
    /// earliest window reset (scoped windows included) nor past [`TRUST_MAX_AGE_S`] from the stored
    /// measurement. The latest hold replaces an earlier one, so `Some(0.0)` lifts it; `None` leaves
    /// it as it is.
    pub fn adopt(
        &self,
        readings: &BTreeMap<String, (Value, f64)>,
        identities: &Identities,
        hold_s: Option<f64>,
    ) -> Result<BTreeSet<String>> {
        let mut adopted = BTreeSet::new();
        if readings.is_empty() {
            return Ok(adopted);
        }
        let now = self.now();
        self.mutate(identities, readings.keys(), |num, row| {
            let (usage, age_s) = &readings[num];
            if adopt_reading(row, usage, *age_s, now) {
                adopted.insert(num.to_owned());
            }
            if let Some(hold_s) = hold_s {
                apply_hold(row, hold_s, now);
            }
        })?;
        Ok(adopted)
    }

    /// Persist the scheduler's per-slot `(nextPollAt, pollIntervalS)`.
    pub fn set_poll_plan(
        &self,
        plans: &BTreeMap<String, PollPlan>,
        identities: &Identities,
    ) -> Result<()> {
        if plans.is_empty() {
            return Ok(());
        }
        self.mutate(identities, plans.keys(), |num, row| {
            let (next_poll_at, interval) = plans[num];
            row.insert("nextPollAt".into(), opt_number(next_poll_at));
            row.insert("pollIntervalS".into(), opt_number(interval));
        })
    }

    /// Lift the dead-token quarantine for slots whose credential was refreshed.
    ///
    /// Called after a re-login/add rewrites a slot's stored credential: the strike count (and the
    /// failure/backoff state riding with it) no longer reflects reality, and the account must
    /// become fetch-eligible again. Also revokes any fetch claim taken with the old credential.
    pub fn clear_dead_token(&self, nums: &[String], identities: &Identities) -> Result<()> {
        if nums.is_empty() {
            return Ok(());
        }
        self.mutate(identities, nums, |_, row| {
            for (key, value) in [
                ("claimId", Value::Null),
                ("claimUntil", number(0.0)),
                ("authDeadStrikes", json!(0)),
                ("struckFingerprint", Value::Null),
                ("consecutiveFailures", json!(0)),
                ("lastError", Value::Null),
                ("backoffUntil", Value::Null),
            ] {
                row.insert(key.into(), value);
            }
        })
    }
}

fn stamp_claim(row: &mut Row, now: f64, claim_id: &str) {
    row.insert("lastAttemptAt".into(), number(now));
    row.insert("claimId".into(), json!(claim_id));
    row.insert("claimUntil".into(), number(now + CLAIM_TTL_S));
}

/// Whether a writer may record into the stored `row`. `expected` is `None` for an unfenced writer,
/// `Some(claim)` for a fenced one (`Some(None)`: the claims map does not name this slot).
fn writer_accepted(
    row: Option<&Value>,
    identity: &Identity,
    expected: Option<Option<&String>>,
    now: f64,
) -> bool {
    let obj = row.and_then(Value::as_object);
    match expected {
        Some(expected) => {
            let Some(expected) = expected else {
                return false;
            };
            matches(row, identity)
                && obj.and_then(|r| r.get("claimId")).and_then(Value::as_str)
                    == Some(expected.as_str())
        }
        // Only a live lease defers an unfenced writer; a crashed claimer's leftover ticket must age
        // out, not block forever.
        None => !obj.is_some_and(|r| {
            r.get("claimId").is_some_and(|c| !c.is_null())
                && now < num(r.get("claimUntil")).unwrap_or(0.0)
        }),
    }
}

fn apply_outcome(row: &mut Row, rec: &FetchRecord, plan: Option<&PollPlan>, now: f64) {
    row.insert("claimId".into(), Value::Null);
    row.insert("claimUntil".into(), number(0.0));
    if rec.sentinel.is_some() {
        if let Some(fp) = &rec.rejected_fp {
            row.insert("rejectedFingerprint".into(), json!(fp));
        }
        return;
    }
    row.insert("lastAttemptAt".into(), number(now));
    match &rec.error {
        None => apply_success(row, rec, plan, now),
        Some(error) => apply_failure(row, rec, error, now),
    }
}

fn apply_success(row: &mut Row, rec: &FetchRecord, plan: Option<&PollPlan>, now: f64) {
    row.insert("lastGood".into(), rec.usage.clone().unwrap_or(Value::Null));
    row.insert("fetchedAt".into(), number(now));
    // Replace the old, possibly due plan in the outcome transaction so no collector can slip into
    // a record→replan gap.
    if let Some((next_poll_at, interval)) = plan {
        row.insert("nextPollAt".into(), opt_number(*next_poll_at));
        row.insert("pollIntervalS".into(), opt_number(*interval));
    }
    row.insert("consecutiveFailures".into(), json!(0));
    row.insert("lastError".into(), Value::Null);
    // A confirmed probe rejection's Retry-After is a deadline no collector may fetch before.
    let backoff = rejection_backoff_until(rec.usage.as_ref(), now);
    row.insert("backoffUntil".into(), opt_number(backoff));
    row.insert("rejectedFingerprint".into(), Value::Null);
    row.insert("authDeadStrikes".into(), json!(0)); // a success proves the token is alive
}

fn apply_failure(row: &mut Row, rec: &FetchRecord, error: &str, now: f64) {
    let failures = int_or_zero(row.get("consecutiveFailures")).saturating_add(1);
    row.insert("consecutiveFailures".into(), json!(failures));
    row.insert("lastError".into(), json!(error));
    let rate_limited = error == "http-429";
    if rate_limited {
        // Kept across later successes: the poll planner floors the cadence while a 429 is recent.
        row.insert("last429At".into(), number(now));
    }
    let wait = failure_backoff_s(failures, rec.retry_after_s, rate_limited);
    row.insert("backoffUntil".into(), number(now + wait));
    // Only a permanent-auth failure advances the dead-token count; a transient error is no
    // evidence either way and must not reset a real tally.
    if PERMANENT_AUTH_ERRORS.contains(&error) {
        let strikes = int_or_zero(row.get("authDeadStrikes")).saturating_add(1);
        row.insert("authDeadStrikes".into(), json!(strikes));
        // Always overwrite: a legacy writer's strike must bind unconditionally, not inherit a stale
        // fingerprint from an earlier, already-healed strike.
        row.insert(
            "struckFingerprint".into(),
            rec.struck_fp.as_ref().map_or(Value::Null, |fp| json!(fp)),
        );
    }
}

/// Replace `lastGood` with an adopted reading when it is newer; returns whether it was.
fn adopt_reading(row: &mut Row, usage: &Value, age_s: f64, now: f64) -> bool {
    let fetched_at = now - age_s.max(0.0);
    if num(row.get("fetchedAt")).is_some_and(|stored| fetched_at <= stored) {
        return false;
    }
    row.insert("lastGood".into(), usage.clone());
    row.insert("fetchedAt".into(), number(fetched_at));
    let deadline = rejection_deadline(Some(usage), now);
    let current = num(row.get("backoffUntil"));
    if let Some(deadline) = deadline.filter(|d| current.is_none_or(|c| *d > c)) {
        row.insert("backoffUntil".into(), number(deadline));
    }
    true
}

fn apply_hold(row: &mut Row, hold_s: f64, now: f64) {
    let stored = num(row.get("fetchedAt"));
    let mut held_until = now + hold_s;
    if let Some(stored) = stored {
        held_until = held_until.min(stored + TRUST_MAX_AGE_S);
    }
    // Every scoped window, not only the models this machine watches: the store does not know them,
    // and a hold lifted early costs one fetch at most.
    if let Some(reset_at) = earliest_reset(row.get("lastGood"), &["all".to_owned()]) {
        held_until = held_until.min(reset_at);
    }
    if held_until > now {
        row.insert("heldUntil".into(), number(held_until));
    } else {
        row.shift_remove("heldUntil");
    }
}

/// The read model of one stored row whose identity matched.
fn entry_of(row: &Row, now: f64, models: &[String]) -> UsageEntry {
    let fetched_at = num(row.get("fetchedAt"));
    let last_good = row.get("lastGood").filter(|v| v.is_object()).cloned();
    let age_s = fetched_at.map(|at| now - at);
    let consecutive_failures = int_or_zero(row.get("consecutiveFailures"));
    let next_poll_at = num(row.get("nextPollAt"));
    let last_attempt_at = num(row.get("lastAttemptAt"));
    let claim_until = num(row.get("claimUntil"));
    let held_until = num(row.get("heldUntil"));
    let last_error = str_field(row, "lastError");
    // A usage-endpoint 429 throttles polling without moving the account's windows: trust up to the
    // earliest future reset (bounded). A non-429 failure is no evidence last_good still holds, so
    // it always uses the general ceiling.
    let within_ceiling = if last_error.as_deref() == Some("http-429") {
        rate_limited_trust_ok(last_good.as_ref(), age_s, now, models)
    } else {
        age_s.is_some_and(|age| age <= TRUST_MAX_AGE_S)
    };
    // Strict < mirrors due_candidate. A live claim keeps the trust bridge up while another
    // collector's result is in flight; a live hold is deliberate staleness as well.
    let trust_extended = within_ceiling
        && (consecutive_failures > 0
            || next_poll_at.is_some_and(|at| now < at)
            || live_claim(claim_until, last_attempt_at, now)
            || held_until.is_some_and(|until| now < until));
    UsageEntry {
        sentinel: None,
        last_good,
        fetched_at,
        age_s,
        last_attempt_at,
        consecutive_failures,
        last_error,
        backoff_until: num(row.get("backoffUntil")),
        next_poll_at,
        poll_interval_s: num(row.get("pollIntervalS")),
        last_429_at: num(row.get("last429At")),
        auth_dead_strikes: int_or_zero(row.get("authDeadStrikes")),
        struck_fingerprint: str_field(row, "struckFingerprint"),
        rejected_fingerprint: str_field(row, "rejectedFingerprint"),
        trust_extended,
        claim_until,
        held_until,
    }
}

#[cfg(test)]
mod tests;
