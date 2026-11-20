//! Switch lines per usage window. Port of `claude_swap/lines.py`.
//!
//! One `autoswitch.threshold` compared against the binding window (the max of the 5h/7d/per-model
//! percentages) cannot express the policy most people want: ~85-90 on the 5-hour window, which
//! recycles in hours, and 97-99 on the weekly ones, which are the budget. So each window gets its
//! own line (`autoswitch.weeklyThreshold` for the windows that reset weekly) and the engine judges
//! every window against its own.
//!
//! Three quantities per account:
//!
//! * RAW headroom (`oauth::account_headroom`, `100 - max pct`) — physical: can the account serve
//!   requests. Untouched by lines.
//! * [`line_headroom`] per window — policy distance on the 5h scale. A window at its line scores
//!   `100 - threshold` whatever its line, an exhausted one 0, linear on each side (anchors:
//!   0% → 100, line → 100 - threshold, 100% → 0). With equal lines it is `100 - pct` bit for bit.
//! * the BINDING window — least line headroom, over-line windows first. Its `line_headroom` is the
//!   account's policy headroom and its `over` the account's verdict.
//!
//! And one margin: [`line_room`], the points left before the nearest line, each window counted in
//! its own points.
//!
//! Pure functions over the windows `oauth::relevant_windows` yields: no settings reads, no I/O.

use crate::oauth::{FIVE_HOUR_LABEL, UsageWindow};

/// The switch line for one window: `weekly_threshold` for every window that resets weekly (7d
/// and per-model), `threshold` for the 5h window and for all of them while no weekly line is set.
pub fn window_line(label: &str, threshold: f64, weekly_threshold: Option<f64>) -> f64 {
    match weekly_threshold {
        Some(weekly) if label != FIVE_HOUR_LABEL => weekly,
        _ => threshold,
    }
}

/// Policy headroom of one window on the 5h scale (module docs).
///
/// The arrangement is load-bearing for exactness at the anchors the engine compares at:
/// `line - pct` is exactly 0.0 at the line, so a window AT its line scores exactly
/// `100 - threshold` (a rescaled `(line - pct) * (threshold / line)` left a 1e-14 residue);
/// `100 - pct` is exactly 0.0 at 100%. Equal lines short-circuit to `100 - pct` so the default
/// configuration is bit-identical to raw headroom.
pub fn line_headroom(pct: f64, line: f64, threshold: f64) -> f64 {
    if line == threshold {
        return 100.0 - pct;
    }
    if pct <= line {
        return (100.0 - threshold) + threshold * (line - pct) / line;
    }
    (100.0 - threshold) * (100.0 - pct) / (100.0 - line)
}

/// Points left before the nearest switch line, each window counted in its own points: the least
/// `line - pct`, negative once one is over. `None` when the account reports no window.
///
/// Not `line_headroom - (100 - threshold)` (that is the room on the 5h scale), nor the binding
/// window's alone (it is chosen on that scale, so the window nearest its line in its own points
/// can be another one).
pub fn line_room(
    windows: &[UsageWindow],
    threshold: f64,
    weekly_threshold: Option<f64>,
) -> Option<f64> {
    windows
        .iter()
        .map(|w| window_line(&w.label, threshold, weekly_threshold) - w.pct)
        .reduce(f64::min)
}

/// The window holding an account back, with the numbers the engine compares: its raw `pct` and
/// its own `line` (so a message can quote "7d 93% < 97%"), `resets_at`, and `line_headroom`.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingWindow {
    pub label: String,
    pub pct: f64,
    pub line: f64,
    pub resets_at: Option<String>,
    pub line_headroom: f64,
}

impl BindingWindow {
    /// At or over its own line — compared directly on the raw pct, never through a rescaled
    /// value, so the verdict cannot drift by a rounding ulp at the line.
    pub fn over(&self) -> bool {
        self.pct >= self.line
    }
}

/// The window with the least policy headroom, over-line windows first; ties keep the first in
/// `relevant_windows` order (5h, 7d, then scoped). `None` when the account reports no window.
///
/// Over-line first makes `binding.over() == any(window at/over its line)` hold by construction.
/// `scale` is the threshold the headroom is measured on (defaults to `threshold`): an account on
/// a line of its own passes that line as `threshold` and the global threshold as `scale`, so
/// accounts on different lines are scored on one ruler. Every caller that selects a window for one
/// account must pass the same `scale` (at a floating-point tie the rounding can pick differently).
pub fn binding_window(
    windows: &[UsageWindow],
    threshold: f64,
    weekly_threshold: Option<f64>,
    scale: Option<f64>,
) -> Option<BindingWindow> {
    let ruler = scale.unwrap_or(threshold);
    let mut best: Option<BindingWindow> = None;
    for w in windows {
        let line = window_line(&w.label, threshold, weekly_threshold);
        let candidate = BindingWindow {
            label: w.label.clone(),
            pct: w.pct,
            line,
            resets_at: w.resets_at.clone(),
            line_headroom: line_headroom(w.pct, line, ruler),
        };
        let better = match &best {
            None => true,
            Some(b) => key_less(&candidate, b),
        };
        if better {
            best = Some(candidate);
        }
    }
    best
}

/// Python's tuple order on `(not over, line_headroom)`, strict.
fn key_less(a: &BindingWindow, b: &BindingWindow) -> bool {
    let (a_under, b_under) = (!a.over(), !b.over());
    if a_under != b_under {
        return !a_under;
    }
    a.line_headroom < b.line_headroom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(label: &str, pct: f64) -> UsageWindow {
        UsageWindow::new(label, pct, None)
    }

    #[test]
    fn window_line_rules() {
        assert_eq!(window_line("5h", 90.0, Some(98.0)), 90.0);
        assert_eq!(window_line("7d", 90.0, Some(98.0)), 98.0);
        assert_eq!(window_line("Fable", 90.0, Some(98.0)), 98.0);
        assert_eq!(window_line("7d", 90.0, None), 90.0);
        assert_eq!(window_line("5h", 90.0, None), 90.0);
    }

    #[test]
    fn equal_lines_are_raw_headroom_bit_for_bit() {
        for pct in [0.0, 12.3, 50.0, 89.99, 90.0, 99.5, 100.0, 120.0] {
            assert_eq!(line_headroom(pct, 90.0, 90.0), 100.0 - pct);
        }
    }

    #[test]
    fn anchors_are_exact_on_the_integer_grid() {
        for t in 1..100 {
            for l in 1..100 {
                let (t, l) = (f64::from(t), f64::from(l));
                assert_eq!(line_headroom(l, l, t), 100.0 - t);
                assert_eq!(line_headroom(100.0, l, t), 0.0);
                assert_eq!(line_headroom(0.0, l, t), 100.0);
            }
        }
    }

    #[test]
    fn anchors_exact_for_decimal_settings() {
        for (t, l) in [(85.5, 97.3), (90.1, 99.9), (60.25, 80.75)] {
            assert_eq!(line_headroom(l, l, t), 100.0 - t);
            assert_eq!(line_headroom(100.0, l, t), 0.0);
        }
    }

    #[test]
    fn values_under_and_over_the_weekly_line() {
        // Under a 98 line on a 90 scale: 10 + 90 * (98 - 49) / 98 = 55.
        assert!((line_headroom(49.0, 98.0, 90.0) - 55.0).abs() < 1e-12);
        // Over: 10 * (100 - 99) / (100 - 98) = 5.
        assert!((line_headroom(99.0, 98.0, 90.0) - 5.0).abs() < 1e-12);
        // A weekly line below the 5h line.
        assert!((line_headroom(30.0, 60.0, 90.0) - 55.0).abs() < 1e-12);
        assert!((line_headroom(80.0, 60.0, 90.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn monotone_in_each_windows_pct() {
        let mut prev = f64::INFINITY;
        for i in 0..=1000 {
            let h = line_headroom(f64::from(i) / 10.0, 97.0, 85.0);
            assert!(h <= prev);
            prev = h;
        }
    }

    #[test]
    fn least_policy_headroom_binds_not_the_highest_pct() {
        // 5h at 80 vs line 90 (headroom 20); 7d at 95 vs line 99 (10 + 90*4/99 ≈ 13.6).
        let b = binding_window(&[w("5h", 80.0), w("7d", 95.0)], 90.0, Some(99.0), None).unwrap();
        assert_eq!(b.label, "7d");
        let b = binding_window(&[w("5h", 85.0), w("7d", 90.0)], 90.0, Some(99.0), None).unwrap();
        assert_eq!(b.label, "5h");
    }

    #[test]
    fn over_is_the_direct_comparison_at_the_line() {
        let b = binding_window(&[w("7d", 97.0)], 90.0, Some(97.0), None).unwrap();
        assert!(b.over());
        assert_eq!(b.line, 97.0);
        assert_eq!(b.line_headroom, 10.0);
    }

    #[test]
    fn ties_keep_the_first_window_and_empty_is_none() {
        let b = binding_window(&[w("5h", 50.0), w("7d", 50.0)], 90.0, None, None).unwrap();
        assert_eq!(b.label, "5h");
        assert_eq!(binding_window(&[], 90.0, None, None), None);
    }

    #[test]
    fn resets_at_travels_with_the_window() {
        let windows = [UsageWindow::new(
            "7d",
            95.0,
            Some("2026-07-05T00:00:00+00:00"),
        )];
        let b = binding_window(&windows, 90.0, None, None).unwrap();
        assert_eq!(b.resets_at.as_deref(), Some("2026-07-05T00:00:00+00:00"));
    }

    #[test]
    fn over_line_windows_bind_first() {
        // 5h at 91 is over its 90 line (headroom 9), 7d at 92 under a 99 line... and a window
        // with less headroom but under its line never outranks an over-line one.
        let b = binding_window(&[w("7d", 98.5), w("5h", 91.0)], 90.0, Some(99.0), None).unwrap();
        assert_eq!(b.label, "5h");
        assert!(b.over());
    }

    #[test]
    fn weekly_unset_reproduces_the_max_pct_rule() {
        let windows = [w("5h", 40.0), w("7d", 70.0), w("Fable", 65.0)];
        let b = binding_window(&windows, 90.0, None, None).unwrap();
        assert_eq!(b.label, "7d");
        assert_eq!(b.line_headroom, 30.0);
    }

    #[test]
    fn a_scale_scores_the_room_on_that_ruler_only() {
        let windows = [w("5h", 30.0)];
        let own = binding_window(&windows, 60.0, None, None).unwrap();
        assert_eq!(own.line_headroom, 70.0);
        let scaled = binding_window(&windows, 60.0, None, Some(90.0)).unwrap();
        // 10 + 90 * (60 - 30) / 60 = 55
        assert!((scaled.line_headroom - 55.0).abs() < 1e-12);
        assert_eq!(scaled.line, own.line);
        assert_eq!(scaled.over(), own.over());
    }

    #[test]
    fn line_room_counts_each_window_in_its_own_points() {
        let windows = [w("5h", 80.0), w("7d", 88.0)];
        assert_eq!(line_room(&windows, 90.0, Some(98.0)), Some(10.0));
        assert_eq!(line_room(&[w("5h", 95.0)], 90.0, None), Some(-5.0));
        assert_eq!(line_room(&[], 90.0, None), None);
    }

    #[test]
    fn line_room_is_not_only_the_binding_window() {
        // 5h 81% is 9 under its 90 line; 7d 88.5% is 9.5 under its 98 line but reads 8.7 on the
        // 5h scale, so the 7d window binds while the nearest line in real points is the 5h one.
        let windows = [w("5h", 81.0), w("7d", 88.5)];
        let b = binding_window(&windows, 90.0, Some(98.0), None).unwrap();
        assert_eq!(b.label, "7d");
        assert_eq!(line_room(&windows, 90.0, Some(98.0)), Some(9.0));
    }

    #[test]
    fn line_room_is_not_the_5h_scale() {
        let windows = [w("5h", 0.0), w("7d", 88.0)];
        let b = binding_window(&windows, 90.0, Some(98.0), None).unwrap();
        assert_eq!(((b.line_headroom - 10.0) * 100.0).round() / 100.0, 9.18);
        assert_eq!(line_room(&windows, 90.0, Some(98.0)), Some(10.0));
        let windows = [w("5h", 0.0), w("7d", 52.0)];
        let b = binding_window(&windows, 60.0, None, Some(90.0)).unwrap();
        assert_eq!(b.line_headroom - 10.0, 12.0);
        assert_eq!(line_room(&windows, 60.0, None), Some(8.0));
    }
}
