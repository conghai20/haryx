//! Ported from `tests/test_poll_policy.py`.

use super::*;
use crate::oauth::iso_utc;
use serde_json::json;

const NOW: f64 = 1_000_000.0;

fn usage(pct: f64, resets_at: Option<&str>) -> Value {
    let mut window = json!({"pct": pct});
    if let Some(r) = resets_at {
        window["resets_at"] = json!(r);
    }
    json!({"five_hour": window, "seven_day": {"pct": 0.0}})
}

fn usage7(pct5: f64, pct7: f64) -> Value {
    json!({"five_hour": {"pct": pct5}, "seven_day": {"pct": pct7}})
}

fn iso_z(ts: f64) -> String {
    iso_utc(ts).unwrap().replace("+00:00", "Z")
}

struct Plan {
    prev_interval_s: Option<f64>,
    prev_usage: Option<Value>,
    new_usage: Option<Value>,
    is_active: bool,
    threshold: f64,
    models: Vec<String>,
    recent_429: bool,
    weekly_threshold: Option<f64>,
    rng: f64,
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            prev_interval_s: None,
            prev_usage: None,
            new_usage: Some(usage(10.0, None)),
            is_active: false,
            threshold: 90.0,
            models: Vec::new(),
            recent_429: false,
            weekly_threshold: None,
            rng: 0.5, // jitter factor exactly 1.0
        }
    }
}

impl Plan {
    fn run(&self) -> (f64, f64) {
        let input = PlanInput {
            prev_interval_s: self.prev_interval_s,
            prev_usage: self.prev_usage.as_ref(),
            new_usage: self.new_usage.as_ref(),
            is_active: self.is_active,
            threshold: self.threshold,
            models: &self.models,
            recent_429: self.recent_429,
            now: NOW,
            weekly_threshold: self.weekly_threshold,
        };
        let r = self.rng;
        plan_after_fetch(&input, &mut || r)
    }

    fn interval(&self) -> f64 {
        self.run().1
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

#[test]
fn first_fetch_uses_defaults() {
    assert_eq!(
        Plan {
            is_active: true,
            ..Default::default()
        }
        .interval(),
        MIN_INTERVAL_S
    );
    assert_eq!(Plan::default().interval(), CANDIDATE_DEFAULT_INTERVAL_S);
}

#[test]
fn unmoved_decays_toward_the_ceiling() {
    let prev = Some(usage(10.0, None));
    let p = |interval: f64, active: bool| Plan {
        prev_interval_s: Some(interval),
        prev_usage: prev.clone(),
        is_active: active,
        ..Default::default()
    };
    assert_eq!(p(300.0, false).interval(), 450.0);
    assert_eq!(p(500.0, false).interval(), CANDIDATE_MAX_INTERVAL_S);
    assert_eq!(p(250.0, true).interval(), ACTIVE_MAX_INTERVAL_S);
}

#[test]
fn movement_halves_floored_at_min() {
    let p = |interval: f64| Plan {
        prev_interval_s: Some(interval),
        prev_usage: Some(usage(10.0, None)),
        new_usage: Some(usage(15.0, None)),
        ..Default::default()
    };
    assert_eq!(p(600.0).interval(), 300.0);
    assert_eq!(p(200.0).interval(), MIN_INTERVAL_S);
}

#[test]
fn sub_delta_wiggle_is_not_movement() {
    let p = Plan {
        prev_interval_s: Some(300.0),
        prev_usage: Some(usage(10.0, None)),
        new_usage: Some(usage(10.5, None)),
        ..Default::default()
    };
    assert_eq!(p.interval(), 450.0);
}

#[test]
fn unknown_pct_uses_the_default() {
    let p = Plan {
        prev_interval_s: Some(600.0),
        new_usage: None,
        ..Default::default()
    };
    assert_eq!(p.interval(), CANDIDATE_DEFAULT_INTERVAL_S);
}

fn urgent() -> Plan {
    Plan {
        prev_interval_s: Some(MIN_INTERVAL_S),
        prev_usage: Some(usage(78.0, None)),
        new_usage: Some(usage(82.0, None)), // moving, inside the 75..90 band
        is_active: true,
        ..Default::default()
    }
}

#[test]
fn urgent_mode() {
    assert_eq!(urgent().interval(), URGENT_INTERVAL_S);
    assert_eq!(
        Plan {
            is_active: false,
            ..urgent()
        }
        .interval(),
        MIN_INTERVAL_S
    );
    assert!(
        Plan {
            new_usage: Some(usage(78.0, None)),
            ..urgent()
        }
        .interval()
            > URGENT_INTERVAL_S
    );
    let below = Plan {
        prev_usage: Some(usage(40.0, None)),
        new_usage: Some(usage(50.0, None)),
        ..urgent()
    };
    assert_eq!(below.interval(), MIN_INTERVAL_S);
    assert_eq!(
        Plan {
            recent_429: true,
            ..urgent()
        }
        .interval(),
        POST_429_MIN_INTERVAL_S
    );
}

#[test]
fn urgent_then_unmoved_snaps_back_to_the_floor() {
    let p = Plan {
        prev_interval_s: Some(URGENT_INTERVAL_S),
        new_usage: Some(usage(78.0, None)),
        ..urgent()
    };
    assert_eq!(p.interval(), MIN_INTERVAL_S);
}

#[test]
fn post_429_floor_and_growth() {
    let base = || Plan {
        recent_429: true,
        prev_usage: Some(usage(10.0, None)),
        ..Default::default()
    };
    assert!(base().interval() >= POST_429_MIN_INTERVAL_S);
    let learned = Plan {
        prev_interval_s: Some(590.0),
        ..base()
    }
    .interval();
    assert!(approx(learned, 590.0 * POST_429_BACKOFF_MULT));
    let from_floor = Plan {
        prev_interval_s: Some(POST_429_MIN_INTERVAL_S),
        ..base()
    }
    .interval();
    assert!(approx(
        from_floor,
        POST_429_MIN_INTERVAL_S * POST_429_BACKOFF_MULT
    ));
    let at_ceiling = Plan {
        prev_interval_s: Some(POST_429_MAX_INTERVAL_S),
        ..base()
    }
    .interval();
    assert_eq!(at_ceiling, POST_429_MAX_INTERVAL_S);
    let no_429 = Plan {
        recent_429: false,
        prev_interval_s: Some(590.0),
        ..base()
    }
    .interval();
    assert_eq!(no_429, CANDIDATE_MAX_INTERVAL_S);
}

fn trajectory(recent_429: bool) -> Vec<f64> {
    let mut prev = None;
    (0..12)
        .map(|_| {
            let interval = Plan {
                recent_429,
                prev_interval_s: prev,
                prev_usage: Some(usage(10.0, None)),
                new_usage: Some(usage(10.0, None)),
                ..Default::default()
            }
            .interval();
            prev = Some(interval);
            interval
        })
        .collect()
}

#[test]
fn sustained_429_grows_the_interval_to_the_wide_ceiling() {
    let traj = trajectory(true);
    assert_eq!(*traj.last().unwrap(), POST_429_MAX_INTERVAL_S);
    for pair in traj.windows(2) {
        assert!(pair[0] <= pair[1]);
        if pair[1] < POST_429_MAX_INTERVAL_S {
            assert!(approx(pair[1], pair[0] * POST_429_BACKOFF_MULT));
        }
    }
    let narrow = trajectory(false);
    assert_eq!(
        narrow.iter().copied().fold(0.0, f64::max),
        CANDIDATE_MAX_INTERVAL_S
    );
}

#[test]
fn reset_capping() {
    let reset = NOW + 90.0;
    let (next, interval) = Plan {
        new_usage: Some(usage(40.0, Some(&iso_z(reset)))),
        ..Default::default()
    }
    .run();
    assert!(approx(next, reset + RESET_SLACK_S));
    assert_eq!(interval, CANDIDATE_DEFAULT_INTERVAL_S);

    let far = NOW + 7200.0;
    for active in [false, true] {
        let p = Plan {
            new_usage: Some(usage(100.0, Some(&iso_z(far)))),
            is_active: active,
            ..Default::default()
        };
        let (next, interval) = p.run();
        assert_eq!(interval, EXHAUSTED_INTERVAL_S);
        assert!(approx(next, NOW + interval));
        assert!(next < far);
    }

    let (next, interval) = Plan {
        new_usage: Some(usage(100.0, Some(&iso_z(reset)))),
        ..Default::default()
    }
    .run();
    assert_eq!(interval, EXHAUSTED_INTERVAL_S);
    assert!(approx(next, reset + RESET_SLACK_S));

    for past in [NOW - 90.0, NOW] {
        let p = Plan {
            new_usage: Some(usage(100.0, Some(&iso_z(past)))),
            ..Default::default()
        };
        let (next, interval) = p.run();
        assert_eq!(interval, EXHAUSTED_INTERVAL_S);
        assert!(approx(next, NOW + interval));
    }
}

#[test]
fn jitter_bounds() {
    let early = Plan {
        rng: 0.0,
        ..Default::default()
    }
    .run()
    .0;
    let late = Plan {
        rng: 1.0,
        ..Default::default()
    }
    .run()
    .0;
    assert!(approx(early, NOW + CANDIDATE_DEFAULT_INTERVAL_S * 0.9));
    assert!(approx(late, NOW + CANDIDATE_DEFAULT_INTERVAL_S * 1.1));
}

/// Relationships the measured rate limit demands of the constants, checked at compile time.
const _: () = {
    assert!(MIN_INTERVAL_S >= 180.0 && SERVE_TTL_S >= 180.0); // 20/hour vs the ~28-30 cap
    assert!(EDGE_BACKOFF_S >= 300.0); // probing slower than capacity frees
    assert!(RECENT_429_WINDOW_S >= 3600.0); // covers the saturation horizon
    assert!(POST_429_MIN_INTERVAL_S >= MIN_INTERVAL_S);
    assert!(POST_429_MAX_INTERVAL_S > CANDIDATE_MAX_INTERVAL_S);
    assert!(ESCALATION_MARGIN_PCT / MOVEMENT_DELTA_PCT < 27.0); // an urgent episode fits the cap
};

fn active(prev: Value, new: Value, threshold: f64, weekly: Option<f64>, models: &[&str]) -> f64 {
    Plan {
        prev_interval_s: Some(MIN_INTERVAL_S),
        prev_usage: Some(prev),
        new_usage: Some(new),
        is_active: true,
        threshold,
        weekly_threshold: weekly,
        models: models.iter().map(|m| m.to_string()).collect(),
        ..Default::default()
    }
    .interval()
}

#[test]
fn per_window_urgency() {
    let w = Some(97.0);
    assert_eq!(
        active(usage(78.0, None), usage(82.0, None), 90.0, None, &[]),
        URGENT_INTERVAL_S
    );
    assert_eq!(
        active(usage7(76.0, 85.0), usage7(78.0, 85.2), 90.0, None, &[]),
        URGENT_INTERVAL_S
    );
    assert_eq!(
        active(usage7(80.0, 94.0), usage7(84.0, 94.0), 85.0, w, &[]),
        URGENT_INTERVAL_S
    );
    assert_eq!(
        active(usage7(20.0, 90.5), usage7(20.0, 96.9), 85.0, w, &[]),
        URGENT_INTERVAL_S
    );
    // A rise outside its own band is not urgent; the flat max decays.
    assert_eq!(
        active(usage7(60.0, 94.0), usage7(62.0, 94.0), 85.0, w, &[]),
        270.0
    );
    // A drop into the band is a rollover, not a burn.
    assert_eq!(
        active(usage(90.0, None), usage(80.0, None), 90.0, None, &[]),
        MIN_INTERVAL_S
    );
    assert_eq!(
        active(usage7(90.0, 80.0), usage7(60.0, 80.0), 90.0, None, &[]),
        MIN_INTERVAL_S
    );
    // A binding flip is not movement.
    assert_eq!(
        active(usage7(84.0, 95.9), usage7(84.1, 95.9), 85.0, w, &[]),
        270.0
    );
    // A window present on one side only is ignored.
    assert_eq!(
        active(
            json!({"five_hour": {"pct": 10.0}}),
            usage7(10.0, 85.0),
            90.0,
            None,
            &[]
        ),
        MIN_INTERVAL_S
    );
}

#[test]
fn candidate_with_weekly_line_decays_or_halves() {
    let p = |interval: f64, prev: Value, new: Value| Plan {
        prev_interval_s: Some(interval),
        prev_usage: Some(prev),
        new_usage: Some(new),
        threshold: 85.0,
        weekly_threshold: Some(97.0),
        ..Default::default()
    };
    assert_eq!(
        p(300.0, usage7(80.0, 94.0), usage7(84.0, 94.0)).interval(),
        450.0
    );
    assert_eq!(
        p(600.0, usage7(20.0, 90.5), usage7(20.0, 96.9)).interval(),
        300.0
    );
}

#[test]
fn a_scoped_window_rises_against_the_weekly_line() {
    let u = |fable: f64| {
        json!({
            "five_hour": {"pct": 10.0},
            "seven_day": {"pct": 10.0},
            "scoped": [{"name": "Fable", "pct": fable}],
        })
    };
    assert_eq!(
        active(u(80.0), u(83.0), 85.0, Some(97.0), &["Fable"]),
        URGENT_INTERVAL_S
    );
    assert_eq!(active(u(80.0), u(83.0), 85.0, Some(97.0), &[]), 270.0);
}

const PNOW: f64 = 1_800_000_000.0;

fn piso(ts: f64) -> String {
    iso_utc(ts).unwrap()
}

#[test]
fn scoped_probe_after_rules() {
    let reset = PNOW + 2.0 * 86400.0;
    assert_eq!(scoped_probe_after(PNOW, Some(&piso(reset)), None), reset);
    assert_eq!(
        scoped_probe_after(PNOW, Some(&piso(reset)), Some(600.0)),
        PNOW + 600.0
    );
    let near = PNOW + 300.0;
    assert_eq!(
        scoped_probe_after(PNOW, Some(&piso(near)), Some(600.0)),
        near
    );
    for retry in [None, Some(0.0), Some(-5.0)] {
        assert_eq!(
            scoped_probe_after(PNOW, None, retry),
            PNOW + SCOPED_PROBE_RETRY_S
        );
    }
    let cap = PNOW + SCOPED_PROBE_MAX_PAUSE_S;
    assert_eq!(
        scoped_probe_after(PNOW, Some(&piso(PNOW + 400.0 * 86400.0)), None),
        cap
    );
    assert_eq!(scoped_probe_after(PNOW, None, Some(1e9)), cap);
}

fn row(probe_after: Option<f64>, resets_at: Option<f64>) -> Value {
    let mut r = json!({"name": "Fable", "pct": 100.0});
    if let Some(p) = probe_after {
        r["probe_after"] = json!(piso(p));
    }
    if let Some(t) = resets_at {
        r["resets_at"] = json!(piso(t));
    }
    r
}

#[test]
fn scoped_probe_paused_rules() {
    let r = row(Some(PNOW + 600.0), Some(PNOW + 86400.0));
    assert!(scoped_probe_paused(&r, PNOW));
    assert!(scoped_probe_paused(&r, PNOW + 599.0));
    assert!(!scoped_probe_paused(&r, PNOW + 600.0));
    assert!(scoped_probe_paused(&row(Some(PNOW + 600.0), None), PNOW));
    assert!(!scoped_probe_paused(&row(None, Some(PNOW + 86400.0)), PNOW));
    assert!(!scoped_probe_paused(
        &row(Some(PNOW + 3600.0), Some(PNOW + 60.0)),
        PNOW + 61.0
    ));
    assert!(!scoped_probe_paused(
        &row(Some(PNOW + SCOPED_PROBE_MAX_PAUSE_S + 1.0), None),
        PNOW
    ));
    assert!(!scoped_probe_paused(
        &json!({"name": "Fable", "pct": 100.0, "probe_after": 5}),
        PNOW
    ));
}

#[test]
fn reset_helpers() {
    let u = json!({
        "five_hour": {"pct": 100.0, "resets_at": "2026-07-05T10:00:00Z"},
        "seven_day": {"pct": 100.0, "resets_at": "2026-07-06T00:00:00+00:00"},
    });
    let later = parse_reset_ts(Some("2026-07-06T00:00:00Z")).unwrap();
    let sooner = parse_reset_ts(Some("2026-07-05T10:00:00Z")).unwrap();
    assert_eq!(limiting_reset_ts(Some(&u), &[]), Some(later));
    assert_eq!(
        earliest_future_reset_ts(Some(&u), sooner - 1.0, &[]),
        Some(sooner)
    );
    assert_eq!(earliest_future_reset_ts(Some(&u), sooner, &[]), Some(later));
    assert_eq!(binding_pct(Some(&u), &[]), Some(100.0));
    assert_eq!(parse_reset_ts(Some("")), None);
    assert_eq!(parse_reset_ts(Some("garbage")), None);
    assert_eq!(parse_reset_ts(None), None);
}
