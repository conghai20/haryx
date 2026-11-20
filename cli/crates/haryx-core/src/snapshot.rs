//! Port of `claude_swap/snapshot_source.py`: the read path for dashboards and GUI shells.
//!
//! Pacing is store-governed (poll plans, freshness, backoff and claims in the usage store), so a
//! dashboard repainting every few seconds and a one-shot `haryx list` produce identical network
//! behavior. [`SnapshotSource::take`] runs the same on-demand pass as `list`; `store_only` reads the
//! store without network eligibility (for shells hosting an auto engine). Blocking: call it from a
//! worker thread, never a UI loop.

use std::collections::HashSet;
use std::sync::Mutex;

use log::debug;

use crate::json_output::USAGE_TOKEN_EXPIRED;
use crate::models::{AccountSnapshot, AccountsSnapshot};
use crate::switcher::Switcher;
use crate::usage_store::UsageEntry;

/// Takes one coherent snapshot per call; never lets a reading go backwards between takes.
pub struct SnapshotSource<'a> {
    switcher: &'a Switcher,
    last: Mutex<Option<AccountsSnapshot>>,
    plans: Mutex<()>,
}

impl<'a> SnapshotSource<'a> {
    pub fn new(switcher: &'a Switcher) -> Self {
        SnapshotSource {
            switcher,
            last: Mutex::new(None),
            plans: Mutex::new(()),
        }
    }

    /// Blocking snapshot pass.
    pub fn take(&self, store_only: bool) -> AccountsSnapshot {
        let none = HashSet::new();
        let fetch = store_only.then_some(&none);
        let snap = self.switcher.accounts_snapshot(fetch);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let snap = match last.as_ref() {
            Some(prev) => reconcile(snap, prev),
            None => snap,
        };
        *last = Some(snap.clone());
        snap
    }

    /// Refresh stored subscription plans unless another refresh is running.
    pub fn refresh_plans(&self) {
        let Ok(_guard) = self.plans.try_lock() else {
            debug!("plan refresh already running");
            return;
        };
        self.switcher.refresh_subscription_plans();
    }
}

fn reconcile(snap: AccountsSnapshot, prev: &AccountsSnapshot) -> AccountsSnapshot {
    let taken_at = snap.taken_at;
    let accounts = snap
        .accounts
        .into_iter()
        .map(|acc| {
            let previous = prev.accounts.iter().find(|p| p.number == acc.number);
            reconcile_account(acc, previous, taken_at)
        })
        .collect();
    AccountsSnapshot { accounts, ..snap }
}

fn identity(acc: &AccountSnapshot) -> (&str, &str, &str) {
    (&acc.email, &acc.org_uuid, &acc.kind)
}

fn reconcile_account(
    acc: AccountSnapshot,
    prev: Option<&AccountSnapshot>,
    taken_at: f64,
) -> AccountSnapshot {
    let Some(prev) = prev.filter(|p| identity(p) == identity(&acc)) else {
        return acc;
    };
    let (prev_fetched, fetched) = (prev.usage.fetched_at, acc.usage.fetched_at);
    if let Some(prev_at) = prev_fetched
        && fetched.is_none_or(|at| at < prev_at)
    {
        return AccountSnapshot {
            usage: with_current_age(&prev.usage, taken_at),
            ..acc
        };
    }
    if acc.usage.sentinel.is_some() {
        return acc;
    }
    if prev.usage.sentinel.as_deref() == Some(USAGE_TOKEN_EXPIRED) && fetched == prev_fetched {
        let usage = UsageEntry {
            sentinel: Some(USAGE_TOKEN_EXPIRED.to_owned()),
            ..acc.usage.clone()
        };
        return AccountSnapshot { usage, ..acc };
    }
    acc
}

fn with_current_age(usage: &UsageEntry, taken_at: f64) -> UsageEntry {
    match usage.fetched_at {
        Some(fetched) => UsageEntry {
            age_s: Some((taken_at - fetched).max(0.0)),
            ..usage.clone()
        },
        None => usage.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(num: &str, fetched_at: Option<f64>, sentinel: Option<&str>) -> AccountSnapshot {
        AccountSnapshot {
            number: num.into(),
            email: format!("{num}@x.io"),
            kind: "oauth".into(),
            usage: UsageEntry {
                fetched_at,
                sentinel: sentinel.map(str::to_owned),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn snap(accounts: Vec<AccountSnapshot>, taken_at: f64) -> AccountsSnapshot {
        AccountsSnapshot {
            active_number: None,
            accounts,
            taken_at,
        }
    }

    #[test]
    fn an_older_reading_never_replaces_a_newer_one() {
        let prev = snap(vec![account("1", Some(100.0), None)], 100.0);
        let next = reconcile(snap(vec![account("1", Some(50.0), None)], 130.0), &prev);
        assert_eq!(next.accounts[0].usage.fetched_at, Some(100.0));
        assert_eq!(next.accounts[0].usage.age_s, Some(30.0));
    }

    #[test]
    fn expired_overlay_sticks_until_a_new_reading() {
        let prev = snap(
            vec![account("1", Some(100.0), Some(USAGE_TOKEN_EXPIRED))],
            100.0,
        );
        let same = reconcile(snap(vec![account("1", Some(100.0), None)], 110.0), &prev);
        assert_eq!(
            same.accounts[0].usage.sentinel.as_deref(),
            Some(USAGE_TOKEN_EXPIRED)
        );
        let newer = reconcile(snap(vec![account("1", Some(120.0), None)], 130.0), &prev);
        assert_eq!(newer.accounts[0].usage.sentinel, None);
    }

    #[test]
    fn a_replaced_slot_is_not_reconciled() {
        let prev = snap(vec![account("1", Some(100.0), None)], 100.0);
        let mut other = account("1", None, None);
        other.email = "else@x.io".into();
        let next = reconcile(snap(vec![other], 110.0), &prev);
        assert_eq!(next.accounts[0].usage.fetched_at, None);
    }
}
