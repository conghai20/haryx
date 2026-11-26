//! Usage state shared with the user's other machines (`crate::usage_share`): the request ledger,
//! merging what another machine wrote, and the local view a machine shares.
//!
//! Every time here is on this machine's clock; the share converts to and from the server's.
//!
//! The ledger (`requests`: `[[time, machine], ...]`) counts usage fetches of the account in the
//! trailing [`LEDGER_WINDOW_S`]. Entries this machine made carry [`LOCAL_MACHINE`]; entries merged
//! from the shared row carry the machine that made them. A slot whose ledger is at
//! [`HOURLY_REQUEST_CAP`] is not eligible to fetch, whatever its plan says.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::policy::num;
use super::{Identities, UsageStore, number, str_field};
use crate::error::Result;
use crate::poll_policy::{HOURLY_REQUEST_CAP, LEDGER_WINDOW_S};

type Row = Map<String, Value>;

/// How far ahead of this machine's clock a shared time may be and still be believed (then taken
/// as now): the server clock is read to the second.
const CLOCK_TOLERANCE_S: f64 = 5.0;

/// The machine id of ledger entries this machine made.
pub const LOCAL_MACHINE: &str = "";

/// One account's shared usage state, on this machine's clock.
///
/// Read from the shared row to merge ([`UsageStore::merge_peers`]), and from the local row to
/// share ([`UsageStore::shared_states`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SharedState {
    pub last_good: Option<Value>,
    pub fetched_at: Option<f64>,
    /// The machine that took `last_good`; `None` for this machine.
    pub fetched_by: Option<String>,
    pub next_poll_at: Option<f64>,
    pub poll_interval_s: Option<f64>,
    pub backoff_until: Option<f64>,
    pub last_429_at: Option<f64>,
    pub last_error: Option<String>,
    /// The lease of the machine fetching now.
    pub claim_until: Option<f64>,
    pub claim_by: Option<String>,
    /// `(time, machine)` of each fetch in the trailing window.
    pub requests: Vec<(f64, String)>,
    /// Machine → until when it reported the account as its active one.
    pub active_on: BTreeMap<String, f64>,
}

/// The ledger of a row: well-formed entries only.
pub(crate) fn ledger(row: &Row) -> Vec<(f64, String)> {
    row.get("requests")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    let at = num(pair.first())?;
                    let by = pair.get(1).and_then(Value::as_str)?;
                    at.is_finite().then(|| (at, by.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Store `entries` as the ledger, keeping only the trailing window, oldest first.
pub(crate) fn write_ledger(row: &mut Row, mut entries: Vec<(f64, String)>, now: f64) {
    entries.retain(|(at, _)| *at > now - LEDGER_WINDOW_S);
    entries.sort_by(|a, b| a.0.total_cmp(&b.0));
    let items: Vec<Value> = entries
        .into_iter()
        .map(|(at, by)| json!([number(at), by]))
        .collect();
    row.insert("requests".into(), Value::Array(items));
}

/// Fetches in the trailing window, every machine counted.
pub(crate) fn requests_in_window(row: &Row, now: f64) -> usize {
    ledger(row)
        .iter()
        .filter(|(at, _)| *at > now - LEDGER_WINDOW_S && *at <= now)
        .count()
}

/// Other machines that fetched in the trailing window.
pub(crate) fn peers_in_window(row: &Row, now: f64) -> usize {
    ledger(row)
        .iter()
        .filter(|(at, by)| *at > now - LEDGER_WINDOW_S && by != LOCAL_MACHINE)
        .map(|(_, by)| by.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

/// Whether the ledger leaves no room for another fetch.
pub(crate) fn ledger_full(row: &Row, now: f64) -> bool {
    requests_in_window(row, now) >= HOURLY_REQUEST_CAP
}

/// Count a fetch this machine is about to make.
pub(crate) fn note_request(row: &mut Row, now: f64) {
    let mut entries = ledger(row);
    entries.push((now, LOCAL_MACHINE.to_owned()));
    write_ledger(row, entries, now);
}

fn later(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// Merge another machine's view of the account into this machine's row.
fn merge_row(row: &mut Row, peer: &SharedState, me: &str, now: f64) {
    let stored = num(row.get("fetchedAt"));
    // Clocks agree only to about a second (the server's `Date` header): a reading slightly ahead
    // of now is taken as now; one far ahead is not believed.
    let fetched_at = peer
        .fetched_at
        .filter(|at| *at <= now + CLOCK_TOLERANCE_S)
        .map(|at| at.min(now));
    let newer =
        peer.last_good.is_some() && fetched_at.is_some_and(|at| stored.is_none_or(|s| at > s));
    // A reading this machine took and shared comes back with its own id: nothing new.
    if newer && peer.fetched_by.as_deref() != Some(me) {
        row.insert(
            "lastGood".into(),
            peer.last_good.clone().unwrap_or(Value::Null),
        );
        row.insert("fetchedAt".into(), json!(fetched_at));
        match &peer.fetched_by {
            Some(by) => row.insert("fetchedBy".into(), json!(by)),
            None => row.shift_remove("fetchedBy"),
        };
        // The schedule is shared: whoever fetched last planned the next poll.
        if let Some(at) = peer.next_poll_at {
            row.insert("nextPollAt".into(), number(at));
        }
        if let Some(interval) = peer.poll_interval_s {
            row.insert("pollIntervalS".into(), number(interval));
        }
    }
    let local_backoff = num(row.get("backoffUntil"));
    if peer
        .backoff_until
        .is_some_and(|until| until > now && local_backoff.is_none_or(|l| until > l))
    {
        row.insert("backoffUntil".into(), json!(peer.backoff_until));
        if let Some(error) = &peer.last_error {
            row.insert("lastError".into(), json!(error));
        }
    }
    if let Some(at) = later(num(row.get("last429At")), peer.last_429_at) {
        row.insert("last429At".into(), number(at));
    }
    // Another machine's live lease keeps this one off until it records.
    if let Some(until) = peer
        .claim_until
        .filter(|until| *until > now && peer.claim_by.as_deref() != Some(me))
    {
        let held = later(num(row.get("heldUntil")), Some(until));
        row.insert("heldUntil".into(), json!(held));
    }
    // The shared ledger is the truth about other machines; this machine's own entries are local.
    let mut entries: Vec<(f64, String)> = ledger(row)
        .into_iter()
        .filter(|(_, by)| by == LOCAL_MACHINE)
        .collect();
    entries.extend(
        peer.requests
            .iter()
            .filter(|(at, by)| by != me && by != LOCAL_MACHINE && *at <= now + CLOCK_TOLERANCE_S)
            .map(|(at, by)| (at.min(now), by.clone())),
    );
    write_ledger(row, entries, now);
    let elsewhere = peer
        .active_on
        .iter()
        .filter(|(by, _)| by.as_str() != me)
        .map(|(_, until)| *until)
        .reduce(f64::max)
        .filter(|until| *until > now);
    match elsewhere {
        Some(until) => row.insert("activeElsewhereUntil".into(), number(until)),
        None => row.shift_remove("activeElsewhereUntil"),
    };
}

/// This machine's row as it is shared.
fn shared_of(row: &Row) -> SharedState {
    SharedState {
        last_good: row.get("lastGood").filter(|v| v.is_object()).cloned(),
        fetched_at: num(row.get("fetchedAt")),
        fetched_by: str_field(row, "fetchedBy"),
        next_poll_at: num(row.get("nextPollAt")),
        poll_interval_s: num(row.get("pollIntervalS")),
        backoff_until: num(row.get("backoffUntil")),
        last_429_at: num(row.get("last429At")),
        last_error: str_field(row, "lastError"),
        claim_until: num(row.get("claimUntil")).filter(|u| *u > 0.0),
        claim_by: None,
        requests: ledger(row),
        active_on: BTreeMap::new(),
    }
}

impl UsageStore {
    /// Merge other machines' shared state, slot → state, into this machine's rows. `me` is this
    /// machine's id: state it wrote itself is recognised and not taken twice.
    pub fn merge_peers(
        &self,
        peers: &BTreeMap<String, SharedState>,
        identities: &Identities,
        me: &str,
    ) -> Result<()> {
        if peers.is_empty() {
            return Ok(());
        }
        let now = self.now();
        self.mutate(identities, peers.keys(), |slot, row| {
            merge_row(row, &peers[slot], me, now);
        })
    }

    /// This machine's shareable state for the given slots (rows of other identities are skipped).
    pub fn shared_states(&self, identities: &Identities) -> BTreeMap<String, SharedState> {
        let rows = self.read_rows();
        identities
            .iter()
            .filter_map(|(num, identity)| {
                let value = rows.get(num)?;
                if !super::matches(Some(value), identity) {
                    return None;
                }
                Some((num.clone(), shared_of(value.as_object()?)))
            })
            .collect()
    }

    /// Give back leases this machine took but will not use (another machine is fetching): the
    /// lease clears and its ledger entry is withdrawn. A lease already replaced is left alone.
    pub fn release(
        &self,
        claims: &BTreeMap<String, String>,
        identities: &Identities,
    ) -> Result<()> {
        if claims.is_empty() {
            return Ok(());
        }
        let now = self.now();
        self.mutate(identities, claims.keys(), |slot, row| {
            if row.get("claimId").and_then(Value::as_str) != Some(claims[slot].as_str()) {
                return;
            }
            let stamped = num(row.get("lastAttemptAt"));
            row.insert("claimId".into(), Value::Null);
            row.insert("claimUntil".into(), number(0.0));
            let mut entries = ledger(row);
            if let Some(pos) = entries
                .iter()
                .rposition(|(at, by)| by == LOCAL_MACHINE && Some(*at) == stamped)
            {
                entries.remove(pos);
            }
            write_ledger(row, entries, now);
        })
    }
}
