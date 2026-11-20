//! Weekly usage pace. Port of `claude_swap/pace.py` (upstream issue #125).
//!
//! A weekly window is "ahead of pace" when the account has used more of it than the fraction of
//! the weekly reset cycle that has elapsed — e.g. 40% used at the 20%-through-the-week mark.
//! Applies only to weekly windows (`seven_day` and every `scoped` per-model window); the 5h window
//! resets too fast for pace to mean anything.
//!
//! [`compute_pace`] is pure: it consumes an already-fetched window and the snapshot's
//! `fetched_at`, never fetches, and has no influence on poll cadence. Elapsed time is measured
//! against `fetched_at` rather than now, so a snapshot served stale is evaluated against the clock
//! it was actually measured at.

use serde_json::Value;

use crate::poll_policy::parse_reset_ts;

/// Weekly windows reset on a fixed 7-day cadence.
pub const WEEKLY_PERIOD_S: f64 = 7.0 * 86400.0;

/// Suppress the marker for this long after a weekly reset: right after reset `expected_pct` is
/// near zero and almost any usage reads as "far ahead" — a false positive.
pub const SUPPRESS_AFTER_RESET_S: f64 = 24.0 * 3600.0;

/// Minimum (actual - expected) percentage-point gap before showing a marker. Below this, "ahead"
/// is normal variance. A flat gap also means the marker cannot fire in the last ~day of the week;
/// deliberate: by then the percentage itself tells the story.
pub const AHEAD_THRESHOLD_PCT: f64 = 15.0;

/// One weekly window's pace at the moment its snapshot was fetched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaceResult {
    /// % of the week's budget "on schedule" usage would be at.
    pub expected_pct: f64,
    /// The window's actual pct.
    pub actual_pct: f64,
    /// Time since this window's current cycle started.
    pub elapsed_s: f64,
    /// The window's full cycle length (e.g. 7 days).
    pub period_s: f64,
    /// `actual_pct - expected_pct >= ahead_threshold_pct`.
    pub ahead: bool,
}

/// The tunables of [`compute_pace_with`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaceParams {
    pub period_s: f64,
    pub suppress_after_reset_s: f64,
    pub ahead_threshold_pct: f64,
}

impl Default for PaceParams {
    fn default() -> Self {
        PaceParams {
            period_s: WEEKLY_PERIOD_S,
            suppress_after_reset_s: SUPPRESS_AFTER_RESET_S,
            ahead_threshold_pct: AHEAD_THRESHOLD_PCT,
        }
    }
}

/// [`compute_pace_with`] with the default weekly parameters.
pub fn compute_pace(window: Option<&Value>, fetched_at: Option<f64>) -> Option<PaceResult> {
    compute_pace_with(window, fetched_at, PaceParams::default())
}

/// Pace for one weekly usage window, or `None` when it isn't computable or meaningful.
///
/// `window` is a stored window object (`{"pct", "resets_at"}`) — `resets_at` is the *next*
/// reset, the only timestamp the usage API provides. The current window's start is derived by
/// rolling `resets_at` by whole periods until it lands at or before `fetched_at`, so this is
/// correct however many cycles old `resets_at` is.
///
/// `None` when the window is missing or unparseable, or elapsed time since the window's start is
/// inside `suppress_after_reset_s`.
pub fn compute_pace_with(
    window: Option<&Value>,
    fetched_at: Option<f64>,
    params: PaceParams,
) -> Option<PaceResult> {
    let window = window?.as_object()?;
    let fetched_at = fetched_at?;
    let pct = match window.get("pct")? {
        Value::Number(n) => n.as_f64()?,
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => return None,
    };
    let next_reset = parse_reset_ts(window.get("resets_at")?.as_str())?;

    // (next_reset - fetched_at) mod period == time until the next reset, folded into
    // [0, period); period minus that is elapsed time since the current window started.
    let remaining = (next_reset - fetched_at).rem_euclid(params.period_s);
    let elapsed = if remaining == 0.0 {
        0.0
    } else {
        params.period_s - remaining
    };
    if elapsed < params.suppress_after_reset_s {
        return None;
    }
    let expected_pct = (elapsed / params.period_s * 100.0).min(100.0);
    Some(PaceResult {
        expected_pct,
        actual_pct: pct,
        elapsed_s: elapsed,
        period_s: params.period_s,
        ahead: pct - expected_pct >= params.ahead_threshold_pct,
    })
}

/// Linear-projection ETA (epoch seconds) for when usage would hit 100%.
///
/// JSON-only: a constant burn rate has wide error bars against bursty usage and would look falsely
/// precise in the UI. `None` when there is no measurable rate.
pub fn projected_exhaustion_ts(pace: &PaceResult, fetched_at: f64) -> Option<f64> {
    if pace.elapsed_s <= 0.0 || pace.actual_pct <= 0.0 {
        return None;
    }
    let rate = pace.actual_pct / pace.elapsed_s;
    if rate <= 0.0 {
        return None;
    }
    let remaining_pct = 100.0 - pace.actual_pct;
    if remaining_pct <= 0.0 {
        return Some(fetched_at);
    }
    Some(fetched_at + remaining_pct / rate)
}

/// Whether, at the current burn rate, usage stays under 100% through reset.
///
/// JSON-only, like [`projected_exhaustion_ts`]. At a constant rate the projected total is
/// `100 × actual / expected`, so this is false exactly when the window is over expected *at all*
/// — the `(ahead)` marker's signal without its threshold. Scripts get the sensitive signal, the
/// UI the noise-gated one. `None` when there is no measurable rate.
pub fn will_last_to_reset(pace: &PaceResult) -> Option<bool> {
    if pace.actual_pct <= 0.0 {
        return Some(true); // no usage yet — nothing to run out of before reset
    }
    if pace.elapsed_s <= 0.0 {
        return None;
    }
    let rate = pace.actual_pct / pace.elapsed_s;
    if rate <= 0.0 {
        return None;
    }
    let projected_total = pace.actual_pct + rate * (pace.period_s - pace.elapsed_s);
    Some(projected_total <= 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::format_countdown;
    use serde_json::json;

    const DAY: f64 = 86400.0;
    // 2026-07-06T00:00:00Z
    const NEXT_RESET: f64 = 1_783_296_000.0;
    const NEXT_RESET_ISO: &str = "2026-07-06T00:00:00+00:00";

    fn window(pct: f64) -> Value {
        json!({"pct": pct, "resets_at": NEXT_RESET_ISO})
    }

    fn at_elapsed(days: f64) -> f64 {
        NEXT_RESET - WEEKLY_PERIOD_S + days * DAY
    }

    #[test]
    fn reset_constant_matches_iso() {
        assert_eq!(parse_reset_ts(Some(NEXT_RESET_ISO)), Some(NEXT_RESET));
        let _ = format_countdown(0.0);
    }

    #[test]
    fn one_day_into_the_week() {
        let p = compute_pace(Some(&window(10.0)), Some(at_elapsed(1.0) + 1.0)).unwrap();
        assert!((p.elapsed_s - (DAY + 1.0)).abs() < 1e-6);
        assert!((p.expected_pct - (DAY + 1.0) / WEEKLY_PERIOD_S * 100.0).abs() < 1e-9);
        assert_eq!(p.actual_pct, 10.0);
        assert!(!p.ahead);
    }

    #[test]
    fn right_at_reset_boundary_is_suppressed() {
        assert_eq!(compute_pace(Some(&window(50.0)), Some(NEXT_RESET)), None);
    }

    #[test]
    fn stale_resets_at_multiple_cycles_in_the_past_still_resolves() {
        let fetched = NEXT_RESET + 3.0 * WEEKLY_PERIOD_S + 2.0 * DAY;
        let p = compute_pace(Some(&window(20.0)), Some(fetched)).unwrap();
        assert!((p.elapsed_s - 2.0 * DAY).abs() < 1e-3);
    }

    #[test]
    fn missing_fields_return_none() {
        let fetched = Some(at_elapsed(3.0));
        assert_eq!(compute_pace(None, fetched), None);
        assert_eq!(compute_pace(Some(&window(10.0)), None), None);
        assert_eq!(
            compute_pace(Some(&json!({"resets_at": NEXT_RESET_ISO})), fetched),
            None
        );
        assert_eq!(compute_pace(Some(&json!({"pct": 10})), fetched), None);
        assert_eq!(
            compute_pace(Some(&json!({"pct": 10, "resets_at": "bad"})), fetched),
            None
        );
        assert_eq!(
            compute_pace(
                Some(&json!({"pct": "10", "resets_at": NEXT_RESET_ISO})),
                fetched
            ),
            None
        );
    }

    #[test]
    fn suppression_window_edges() {
        let inside = at_elapsed(1.0) - 1.0;
        assert_eq!(compute_pace(Some(&window(50.0)), Some(inside)), None);
        assert!(compute_pace(Some(&window(50.0)), Some(at_elapsed(1.0) + 1.0)).is_some());
    }

    #[test]
    fn ahead_threshold() {
        // 2 days in → expected ≈ 28.57%.
        let fetched = Some(at_elapsed(2.0));
        assert!(compute_pace(Some(&window(50.0)), fetched).unwrap().ahead);
        let near = compute_pace(Some(&window(35.0)), fetched).unwrap();
        assert!(!near.ahead);
        assert!(!compute_pace(Some(&window(5.0)), fetched).unwrap().ahead);
    }

    #[test]
    fn projected_exhaustion() {
        let fetched = at_elapsed(2.0);
        let p = compute_pace(Some(&window(40.0)), Some(fetched)).unwrap();
        // 40% in 2 days → 60% more takes 3 days.
        let eta = projected_exhaustion_ts(&p, fetched).unwrap();
        assert!((eta - (fetched + 3.0 * DAY)).abs() < 1e-3);
        let full = compute_pace(Some(&window(100.0)), Some(fetched)).unwrap();
        assert_eq!(projected_exhaustion_ts(&full, fetched), Some(fetched));
        let zero = compute_pace(Some(&window(0.0)), Some(fetched)).unwrap();
        assert_eq!(projected_exhaustion_ts(&zero, fetched), None);
    }

    #[test]
    fn will_last() {
        let fetched = Some(at_elapsed(3.5)); // half the week
        assert_eq!(
            will_last_to_reset(&compute_pace(Some(&window(40.0)), fetched).unwrap()),
            Some(true)
        );
        assert_eq!(
            will_last_to_reset(&compute_pace(Some(&window(60.0)), fetched).unwrap()),
            Some(false)
        );
        assert_eq!(
            will_last_to_reset(&compute_pace(Some(&window(0.0)), fetched).unwrap()),
            Some(true)
        );
        assert_eq!(
            will_last_to_reset(&compute_pace(Some(&window(50.0)), fetched).unwrap()),
            Some(true)
        );
    }

    #[test]
    fn slightly_ahead_reads_wont_last_with_no_marker() {
        let p = compute_pace(Some(&window(55.0)), Some(at_elapsed(3.5))).unwrap();
        assert!(!p.ahead);
        assert_eq!(will_last_to_reset(&p), Some(false));
    }
}
