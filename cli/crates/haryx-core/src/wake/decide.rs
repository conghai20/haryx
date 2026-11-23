//! The wake decision: a pure function of one account's reading, the clock and the settings.

use super::config::{QuietHours, WakeConfig};
use super::state::AccountState;

/// Seconds after `resets_at` before a window counts as over, so a wake never lands before the
/// server rolls the window.
pub const GRACE_S: f64 = 5.0;
/// A wake that did not start a window is not repeated for this long.
pub const RETRY_AFTER_S: f64 = 15.0 * 60.0;

/// What one reading says about an account's windows.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowRead {
    /// When the current 5-hour window resets (epoch seconds), when one is reported.
    pub five_hour_resets_at: Option<f64>,
    /// 7-day utilization in percent, when reported.
    pub seven_day_pct: Option<f64>,
}

/// Why a due account was not woken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// A window is already running (the account was used after the reset).
    AlreadyActive,
    QuietHours,
    WeeklyCap,
    /// A wake was sent recently and did not start a window yet.
    RecentlyWoken,
    /// The login is expired or was refused.
    Login,
    /// No usage reading could be had.
    NoReading,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::AlreadyActive => "already-active",
            SkipReason::QuietHours => "quiet-hours",
            SkipReason::WeeklyCap => "weekly-cap",
            SkipReason::RecentlyWoken => "recently-woken",
            SkipReason::Login => "login",
            SkipReason::NoReading => "no-reading",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    /// The window is still running; check again at `next` (epoch seconds).
    NotDue {
        next: f64,
    },
    Skip(SkipReason),
    Wake,
}

/// The clock as the decision needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Now {
    pub epoch: f64,
    /// Local minutes since midnight, for quiet hours.
    pub local_minute: u16,
}

/// Whether the 5-hour window of `read` is still running at `now`.
pub fn window_running(read: &WindowRead, now: f64) -> Option<f64> {
    read.five_hour_resets_at
        .filter(|resets| now < resets + GRACE_S)
}

/// Decide for one opted-in account. `read` is the freshest reading on hand; the caller refreshes
/// it before acting on a [`Decision::Wake`] from a stored one.
pub fn decide(read: &WindowRead, now: Now, config: &WakeConfig, state: &AccountState) -> Decision {
    if let Some(resets) = window_running(read, now.epoch) {
        return Decision::NotDue {
            next: resets + GRACE_S,
        };
    }
    if config
        .quiet_hours
        .is_some_and(|q| q.contains(now.local_minute))
    {
        return Decision::Skip(SkipReason::QuietHours);
    }
    if read
        .seven_day_pct
        .is_some_and(|pct| pct >= config.weekly_cap_pct)
    {
        return Decision::Skip(SkipReason::WeeklyCap);
    }
    if state
        .last_wake_at
        .is_some_and(|at| now.epoch - at < RETRY_AFTER_S)
    {
        return Decision::Skip(SkipReason::RecentlyWoken);
    }
    Decision::Wake
}

impl QuietHours {
    /// Whether `minute` (local minutes since midnight) falls inside, wrapping past midnight.
    pub fn contains(self, minute: u16) -> bool {
        let (start, end) = (self.start, self.end);
        match start.cmp(&end) {
            std::cmp::Ordering::Less => minute >= start && minute < end,
            std::cmp::Ordering::Greater => minute >= start || minute < end,
            std::cmp::Ordering::Equal => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: f64 = 1_800_000_000.0;

    fn now(epoch: f64, local_minute: u16) -> Now {
        Now {
            epoch,
            local_minute,
        }
    }

    fn cfg() -> WakeConfig {
        WakeConfig::default()
    }

    #[test]
    fn running_window_is_not_due_until_reset_plus_grace() {
        let read = WindowRead {
            five_hour_resets_at: Some(T + 100.0),
            seven_day_pct: Some(10.0),
        };
        let d = decide(&read, now(T, 600), &cfg(), &AccountState::default());
        assert_eq!(d, Decision::NotDue { next: T + 105.0 });
    }

    #[test]
    fn inside_grace_is_still_not_due() {
        let read = WindowRead {
            five_hour_resets_at: Some(T - 2.0),
            seven_day_pct: None,
        };
        let d = decide(&read, now(T, 600), &cfg(), &AccountState::default());
        assert!(matches!(d, Decision::NotDue { .. }));
    }

    #[test]
    fn passed_reset_wakes() {
        let read = WindowRead {
            five_hour_resets_at: Some(T - 60.0),
            seven_day_pct: Some(40.0),
        };
        let d = decide(&read, now(T, 600), &cfg(), &AccountState::default());
        assert_eq!(d, Decision::Wake);
    }

    #[test]
    fn no_window_reported_wakes() {
        let d = decide(
            &WindowRead::default(),
            now(T, 600),
            &cfg(),
            &AccountState::default(),
        );
        assert_eq!(d, Decision::Wake);
    }

    #[test]
    fn quiet_hours_skip() {
        let config = WakeConfig {
            quiet_hours: Some(QuietHours {
                start: 60,
                end: 360,
            }),
            ..cfg()
        };
        let d = decide(
            &WindowRead::default(),
            now(T, 120),
            &config,
            &AccountState::default(),
        );
        assert_eq!(d, Decision::Skip(SkipReason::QuietHours));
    }

    #[test]
    fn weekly_cap_skips_at_threshold() {
        let read = WindowRead {
            five_hour_resets_at: None,
            seven_day_pct: Some(90.0),
        };
        let d = decide(&read, now(T, 600), &cfg(), &AccountState::default());
        assert_eq!(d, Decision::Skip(SkipReason::WeeklyCap));
    }

    #[test]
    fn recent_wake_is_not_repeated() {
        let state = AccountState {
            last_wake_at: Some(T - 60.0),
            ..AccountState::default()
        };
        let d = decide(&WindowRead::default(), now(T, 600), &cfg(), &state);
        assert_eq!(d, Decision::Skip(SkipReason::RecentlyWoken));
    }

    #[test]
    fn old_wake_allows_another() {
        let state = AccountState {
            last_wake_at: Some(T - RETRY_AFTER_S - 1.0),
            ..AccountState::default()
        };
        let d = decide(&WindowRead::default(), now(T, 600), &cfg(), &state);
        assert_eq!(d, Decision::Wake);
    }

    #[test]
    fn quiet_hours_wrap_past_midnight() {
        let q = QuietHours {
            start: 23 * 60,
            end: 6 * 60,
        };
        assert!(q.contains(23 * 60 + 30));
        assert!(q.contains(2 * 60));
        assert!(!q.contains(6 * 60));
        assert!(!q.contains(12 * 60));
    }

    #[test]
    fn equal_quiet_bounds_are_empty() {
        let q = QuietHours { start: 60, end: 60 };
        assert!(!q.contains(60));
    }
}
