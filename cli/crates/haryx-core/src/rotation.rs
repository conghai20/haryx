//! Will the week last? The rotation verdict behind the apps' Usage page.
//!
//! Auto-switch spends the weekly budget of every account it may pick, one after another, so the
//! question a user with several accounts asks is not "how full is this one" but "do the accounts
//! auto-switch can use carry me to their resets". This module answers it from readings the caller
//! already has: each account's weekly %, when its week resets, and its weekly switch line.
//!
//! The model is a shared pool. Its level is the room left under each eligible account's weekly
//! line. It drains at the user's whole burn rate: every account's weekly % divided by the time its
//! current week has run, summed (use keeps landing somewhere, eligible or not). When an eligible
//! account's week resets, the pool gains back the part of its line it had used at the start, a
//! deliberate underestimate: whatever the pool drew from that account since is not counted, so the
//! verdict errs towards "runs dry". The run covers one week from now; by then every account has
//! reset once.
//!
//! Pure: no I/O, no clock. The caller passes `now`.

use serde_json::{Value, json};

use crate::json_output::timestamp;
use crate::pace::WEEKLY_PERIOD_S;

/// A week must have run this long before its % says anything about a rate.
pub const MIN_ELAPSED_S: f64 = 3600.0;

/// One account's weekly reading, as the verdict sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct RotationInput {
    pub number: i64,
    /// Auto-switch may land on it: in rotation, not quarantined, a subscription account.
    pub eligible: bool,
    /// The weekly window, when read: `(pct, resets_at epoch seconds)`.
    pub weekly: Option<(f64, f64)>,
    /// The weekly switch line auto-switch leaves this account at.
    pub line: f64,
}

/// The verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct Rotation {
    /// Eligible accounts with a weekly reading, in input order.
    pub accounts: Vec<i64>,
    /// Eligible accounts without one: the verdict leaves them out.
    pub unknown: Vec<i64>,
    /// Room left under the eligible accounts' weekly lines, in % of one account's week.
    pub headroom_pct: f64,
    /// The whole fleet's weekly use per hour, in % of one account's week.
    pub burn_pct_per_hour: f64,
    /// The earliest weekly reset among the counted accounts.
    pub next_reset_at: Option<f64>,
    /// When the pool empties within the coming week; `None`: it lasts.
    pub runs_dry_at: Option<f64>,
}

impl Rotation {
    pub fn lasts(&self) -> bool {
        self.runs_dry_at.is_none()
    }

    /// The `rotation` object of `list --json`.
    pub fn to_json(&self) -> Value {
        json!({
            "accounts": self.accounts,
            "unknown": self.unknown,
            "headroomPct": round1(self.headroom_pct),
            "burnPctPerHour": (self.burn_pct_per_hour * 100.0).round() / 100.0,
            "nextResetAt": self.next_reset_at.map(timestamp),
            "runsDryAt": self.runs_dry_at.map(timestamp),
            "lasts": self.lasts(),
        })
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// How long the current week has run at `now`, given its next reset.
fn elapsed(resets_at: f64, now: f64) -> f64 {
    let remaining = (resets_at - now).rem_euclid(WEEKLY_PERIOD_S);
    if remaining == 0.0 {
        0.0
    } else {
        WEEKLY_PERIOD_S - remaining
    }
}

/// The next reset strictly after `now`, rolling a stale `resets_at` forward by whole weeks.
fn next_reset(resets_at: f64, now: f64) -> f64 {
    now + (resets_at - now).rem_euclid(WEEKLY_PERIOD_S)
}

/// The verdict, or `None` when no eligible account has a weekly reading. `measured_burn_per_s` is the
/// fleet's rate from the kept readings (`usage_history::quota::weekly_burn_per_s`); without it each
/// account's average since its reset stands in, which overstates a burst that has since moved on.
pub fn rotation(
    inputs: &[RotationInput],
    measured_burn_per_s: Option<f64>,
    now: f64,
) -> Option<Rotation> {
    let averaged = || -> f64 {
        inputs
            .iter()
            .filter_map(|i| i.weekly)
            .map(|(pct, resets_at)| {
                let run = elapsed(resets_at, now);
                if run < MIN_ELAPSED_S {
                    0.0
                } else {
                    pct.max(0.0) / run
                }
            })
            .sum()
    };
    let burn_per_s = measured_burn_per_s.unwrap_or_else(averaged);
    let counted: Vec<(i64, f64, f64, f64)> = inputs
        .iter()
        .filter(|i| i.eligible)
        .filter_map(|i| {
            let (pct, resets_at) = i.weekly?;
            Some((i.number, pct, next_reset(resets_at, now), i.line))
        })
        .collect();
    if counted.is_empty() {
        return None;
    }
    let unknown = inputs
        .iter()
        .filter(|i| i.eligible && i.weekly.is_none())
        .map(|i| i.number)
        .collect();
    let headroom: f64 = counted
        .iter()
        .map(|(_, pct, _, line)| (line - pct).max(0.0))
        .sum();
    let mut resets: Vec<(f64, f64)> = counted
        .iter()
        .map(|(_, pct, at, line)| (*at, pct.min(*line).max(0.0)))
        .collect();
    resets.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(Rotation {
        accounts: counted.iter().map(|c| c.0).collect(),
        unknown,
        headroom_pct: headroom,
        burn_pct_per_hour: burn_per_s * 3600.0,
        next_reset_at: resets.first().map(|r| r.0),
        runs_dry_at: runs_dry_at(headroom, burn_per_s, &resets, now),
    })
}

/// Drain the pool at `burn_per_s` through the resets (`(at, regained)`, sorted); when it empties.
fn runs_dry_at(headroom: f64, burn_per_s: f64, resets: &[(f64, f64)], now: f64) -> Option<f64> {
    if burn_per_s <= 0.0 {
        return None;
    }
    let horizon = now + WEEKLY_PERIOD_S;
    let mut level = headroom;
    let mut at = now;
    let steps = resets
        .iter()
        .copied()
        .filter(|(t, _)| *t <= horizon)
        .chain(std::iter::once((horizon, 0.0)));
    for (reset_at, regained) in steps {
        let spend = burn_per_s * (reset_at - at);
        if spend >= level {
            return Some(at + level / burn_per_s);
        }
        level = level - spend + regained;
        at = reset_at;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 3600.0;
    const NOW: f64 = 1_000_000.0;

    fn account(number: i64, pct: f64, resets_in_h: f64) -> RotationInput {
        RotationInput {
            number,
            eligible: true,
            weekly: Some((pct, NOW + resets_in_h * H)),
            line: 100.0,
        }
    }

    #[test]
    fn none_when_no_eligible_account_has_a_reading() {
        let mut off = account(1, 50.0, 24.0);
        off.eligible = false;
        let unread = RotationInput {
            weekly: None,
            ..account(2, 0.0, 0.0)
        };
        assert_eq!(rotation(&[off, unread], None, NOW), None);
    }

    #[test]
    fn idle_fleet_lasts() {
        let r = rotation(&[account(1, 0.0, 100.0)], None, NOW).unwrap();
        assert!(r.lasts());
        assert_eq!(r.burn_pct_per_hour, 0.0);
        assert_eq!(r.headroom_pct, 100.0);
    }

    #[test]
    fn one_account_burning_fast_runs_dry_before_its_reset() {
        // 84 h into the week at 70%: 0.8333 %/h, 30% left lasts 36 h; reset is 84 h away.
        let r = rotation(&[account(1, 70.0, 84.0)], None, NOW).unwrap();
        let dry = r.runs_dry_at.unwrap();
        assert!((dry - (NOW + 36.0 * H)).abs() < 1.0, "{dry}");
        assert_eq!(r.next_reset_at, Some(NOW + 84.0 * H));
    }

    #[test]
    fn a_second_account_carries_the_week() {
        let r = rotation(&[account(1, 70.0, 84.0), account(2, 0.0, 120.0)], None, NOW).unwrap();
        assert!(r.lasts(), "{r:?}");
        assert_eq!(r.accounts, vec![1, 2]);
        assert_eq!(r.headroom_pct, 130.0);
    }

    #[test]
    fn ineligible_use_still_drains_the_pool() {
        let mut held = account(2, 90.0, 24.0);
        held.eligible = false;
        let alone = rotation(&[account(1, 10.0, 24.0)], None, NOW).unwrap();
        let with_held = rotation(&[account(1, 10.0, 24.0), held], None, NOW).unwrap();
        assert!(with_held.burn_pct_per_hour > alone.burn_pct_per_hour);
        assert_eq!(with_held.accounts, vec![1]);
    }

    #[test]
    fn the_switch_line_caps_headroom() {
        let mut lined = account(1, 80.0, 100.0);
        lined.line = 90.0;
        let r = rotation(&[lined], None, NOW).unwrap();
        assert_eq!(r.headroom_pct, 10.0);
    }

    #[test]
    fn a_reset_refills_the_pool() {
        // 60% used 144 h in (0.4167 %/h), 40 left lasts 96 h; the reset at 24 h regains 60.
        let r = rotation(&[account(1, 60.0, 24.0)], None, NOW).unwrap();
        assert!(r.lasts(), "{r:?}");
    }

    #[test]
    fn a_fresh_week_gives_no_rate() {
        // Reset 30 min ago: too soon to read a rate from.
        let r = rotation(&[account(1, 20.0, 167.5)], None, NOW).unwrap();
        assert_eq!(r.burn_pct_per_hour, 0.0);
        assert!(r.lasts());
    }

    #[test]
    fn stale_reset_rolls_forward() {
        let r = rotation(&[account(1, 10.0, -1.0)], None, NOW).unwrap();
        assert_eq!(r.next_reset_at, Some(NOW + 167.0 * H));
    }

    #[test]
    fn a_measured_rate_wins_over_the_averages() {
        let r = rotation(&[account(1, 70.0, 84.0)], Some(0.0), NOW).unwrap();
        assert!(r.lasts());
    }

    #[test]
    fn json_shape() {
        let r = rotation(&[account(1, 70.0, 84.0)], None, NOW).unwrap();
        let v = r.to_json();
        assert_eq!(v["lasts"], json!(false));
        assert_eq!(v["accounts"], json!([1]));
        assert!(v["runsDryAt"].is_string());
        assert!(v["nextResetAt"].is_string());
    }
}
