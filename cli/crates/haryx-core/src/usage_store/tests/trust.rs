//! Trust, staleness and backoff.

use super::*;

// --- schema -------------------------------------------------------------------------------------

#[test]
fn empty_when_missing() {
    let f = fixture();
    assert_eq!(f.entry("1"), UsageEntry::default());
    assert!(f.entry("1").decision_value().is_none());
}

#[test]
fn versionless_legacy_snapshot_ignored() {
    let f = fixture();
    f.write_raw(&json!({"timestamp": 123, "data": {"1": usage()}}).to_string());
    assert!(f.entry("1").last_good.is_none());
}

#[test]
fn corrupt_file_ignored() {
    let f = fixture();
    f.write_raw("{not json");
    assert_eq!(f.entry("1"), UsageEntry::default());
}

#[test]
fn round_trip() {
    let f = fixture();
    f.record("1", ok(usage()));
    let raw = f.raw();
    assert_eq!(raw["schemaVersion"], json!(2));
    let row = &raw["accounts"]["1"];
    assert_eq!(row["email"], json!("a@x.com"));
    assert_eq!(row["organizationUuid"], json!(""));
    assert_eq!(row["lastGood"], usage());
    assert_eq!(row["fetchedAt"].as_f64(), Some(f.clock.now()));
    let entry = f.entry("1");
    assert_eq!(entry.last_good, Some(usage()));
    assert_eq!(entry.age_s, Some(0.0));
    assert_eq!(decision_usage(&entry), Some(usage()));
}

#[test]
fn file_is_python_indented_json() {
    let f = fixture();
    f.record("1", ok(usage()));
    let text = std::fs::read_to_string(&f.store.path).unwrap();
    assert!(text.starts_with("{\n  \"schemaVersion\": 2,\n  \"accounts\": {"));
}

// --- stale on error -----------------------------------------------------------------------------

#[test]
fn failure_preserves_last_good() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.clock.advance(60.0);
    f.record("1", err("http-429"));
    let e = f.entry("1");
    assert_eq!(e.last_good, Some(usage()));
    assert_eq!(e.age_s, Some(60.0));
    assert_eq!(e.last_error.as_deref(), Some("http-429"));
    assert_eq!(e.consecutive_failures, 1);
    assert_eq!(decision_usage(&e), Some(usage()));
}

#[test]
fn too_stale_is_unknown_for_decisions() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.clock.advance(STALE_OK_S + 1.0);
    let e = f.entry("1");
    assert!(e.decision_value().is_none());
    assert_eq!(e.last_good, Some(usage()));
    assert_eq!(e.age_s, Some(STALE_OK_S + 1.0));
}

#[test]
fn success_clears_failure_state() {
    let f = fixture();
    f.record("1", err("timeout"));
    f.clock.advance(5.0);
    f.record("1", ok(usage()));
    let e = f.entry("1");
    assert_eq!(e.consecutive_failures, 0);
    assert!(e.last_error.is_none());
    assert!(e.backoff_until.is_none());
    assert_eq!(decision_usage(&e), Some(usage()));
}

#[test]
fn success_with_no_windows() {
    let f = fixture();
    f.record("1", FetchRecord::success(None));
    let e = f.entry("1");
    assert!(e.last_error.is_none());
    assert!(e.fetched_at.is_some());
    assert!(e.decision_value().is_none());
}

// --- extended trust -----------------------------------------------------------------------------

#[test]
fn in_backoff_past_stale_ok_is_still_trusted() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.clock.advance(STALE_OK_S);
    f.record("1", err_after("http-429", 480.0));
    f.clock.advance(60.0);
    let e = f.entry("1");
    assert!(e.age_s.unwrap() > STALE_OK_S);
    assert!(e.in_backoff(f.clock.now()));
    assert!(e.trust_extended);
    assert_eq!(decision_usage(&e), Some(usage()));
}

#[test]
fn failure_state_after_backoff_expiry_is_still_trusted() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.clock.advance(60.0);
    f.record("1", err("timeout"));
    f.clock.advance(BACKOFF_BASE_S + STALE_OK_S);
    let e = f.entry("1");
    assert!(!e.in_backoff(f.clock.now()));
    assert_eq!(decision_usage(&e), Some(usage()));
}

#[test]
fn within_poll_plan_past_stale_ok_is_trusted() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.plan("1", (Some(f.clock.now() + 600.0), Some(600.0)));
    f.clock.advance(400.0);
    assert_eq!(decision_usage(&f.entry("1")), Some(usage()));
    f.clock.advance(250.0);
    assert!(f.entry("1").decision_value().is_none());
}

#[test]
fn trust_ceiling_wins_over_non_429_failure_state() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.record("1", err("timeout"));
    f.clock.advance(TRUST_MAX_AGE_S + 1.0);
    f.record("1", err("timeout"));
    let e = f.entry("1");
    assert_eq!(e.consecutive_failures, 2);
    assert!(e.decision_value().is_none());
}

#[test]
fn staleness_429_trusted_until_window_reset() {
    let f = fixture();
    let ahead = (TRUST_MAX_AGE_S + RATE_LIMIT_TRUST_MAX_AGE_S) / 2.0;
    let u = usage_resetting(f.clock.now(), Some(ahead), Some(ahead));
    f.record("1", ok(u.clone()));
    f.record("1", err("http-429"));
    f.clock.advance(TRUST_MAX_AGE_S + 1.0);
    f.record("1", err("http-429"));
    let e = f.entry("1");
    assert!(e.trust_extended);
    assert_eq!(decision_usage(&e), Some(u));
}

#[test]
fn staleness_429_expires_at_window_reset() {
    let f = fixture();
    let u = usage_resetting(f.clock.now(), Some(600.0), Some(600.0));
    f.record("1", ok(u.clone()));
    f.record("1", err("http-429"));
    f.clock.advance(601.0);
    f.record("1", err("http-429"));
    let e = f.entry("1");
    assert!(e.decision_value().is_none());
    assert_eq!(e.last_good, Some(u));
}

#[test]
fn staleness_429_without_reset_info_falls_back_to_ceiling() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.record("1", err("http-429"));
    f.clock.advance(TRUST_MAX_AGE_S + 1.0);
    f.record("1", err("http-429"));
    assert_eq!(decision_usage(&f.entry("1")), Some(usage()));
    f.clock.advance(RATE_LIMIT_TRUST_MAX_AGE_S);
    f.record("1", err("http-429"));
    assert!(f.entry("1").decision_value().is_none());
}

// --- rate-limit trust bounds --------------------------------------------------------------------

fn rate_limited_case(five: Option<f64>, seven: Option<f64>, advance: f64) -> Option<Value> {
    let f = fixture();
    let u = usage_resetting(f.clock.now(), five, seven);
    f.record("1", ok(u));
    f.record("1", err("http-429"));
    f.clock.advance(advance);
    f.record("1", err("http-429"));
    decision_usage(&f.entry("1"))
}

#[test]
fn far_future_reset_is_clamped_to_the_ceiling() {
    let far = 10.0 * 365.0 * 24.0 * 3600.0;
    assert!(rate_limited_case(Some(far), Some(far), RATE_LIMIT_TRUST_MAX_AGE_S + 1.0).is_none());
}

#[test]
fn trust_keys_on_earliest_future_reset() {
    assert!(rate_limited_case(Some(600.0), Some(100.0 * 3600.0), 601.0).is_none());
}

#[test]
fn partial_metadata_still_bounded_by_ceiling() {
    let r = rate_limited_case(None, Some(100.0 * 3600.0), RATE_LIMIT_TRUST_MAX_AGE_S + 1.0);
    assert!(r.is_none());
}

#[test]
fn ceiling_wins_when_reset_is_beyond_it() {
    let far = RATE_LIMIT_TRUST_MAX_AGE_S * 3.0;
    assert!(rate_limited_case(Some(far), Some(far), RATE_LIMIT_TRUST_MAX_AGE_S - 60.0).is_some());
    assert!(rate_limited_case(Some(far), Some(far), RATE_LIMIT_TRUST_MAX_AGE_S + 60.0).is_none());
}

// --- backoff ------------------------------------------------------------------------------------

#[test]
fn exponential_backoff() {
    let f = fixture();
    for (i, want) in [30.0, 60.0, 120.0, 240.0, 480.0, 600.0, 600.0]
        .iter()
        .enumerate()
    {
        f.record("1", err("http-500"));
        let e = f.entry("1");
        assert_eq!(e.consecutive_failures, i as i64 + 1);
        assert!(approx(e.backoff_until.unwrap(), f.clock.now() + want));
        f.clock.advance(want + 1.0);
    }
}

#[test]
fn backoff_cap_and_no_overflow() {
    assert_eq!(failure_backoff_s(50, None, true), BACKOFF_CAP_S);
    assert_eq!(failure_backoff_s(1025, None, true), BACKOFF_CAP_S);
    assert_eq!(failure_backoff_s(10_000, Some(90.0), true), BACKOFF_CAP_S);
    assert_eq!(failure_backoff_s(i64::MAX, None, true), BACKOFF_CAP_S);
}

#[test]
fn record_failure_on_saturated_counter() {
    let f = fixture();
    f.write_raw(
        &json!({"schemaVersion": 2, "accounts": {"1": {
            "email": "a@x.com", "consecutiveFailures": 1024, "lastError": "refresh-failed"}}})
        .to_string(),
    );
    f.record("1", err("refresh-failed"));
    let e = f.entry("1");
    assert_eq!(e.consecutive_failures, 1025);
    assert!(approx(
        e.backoff_until.unwrap(),
        f.clock.now() + BACKOFF_CAP_S
    ));
}

#[test]
fn retry_after_is_the_floor() {
    let f = fixture();
    f.record("1", err_after("http-429", 90.0));
    let e = f.entry("1");
    assert!(approx(e.backoff_until.unwrap(), f.clock.now() + 90.0));
    assert!(e.in_backoff(f.clock.now() + 89.0));
    assert!(!e.in_backoff(f.clock.now() + 91.0));
}

#[test]
fn own_curve_may_exceed_retry_after() {
    assert_eq!(failure_backoff_s(5, Some(10.0), true), 480.0);
}

#[test]
fn edge_429_backoff_floors_at_edge_backoff() {
    let f = fixture();
    for want in [300.0, 300.0, 300.0, 300.0, 480.0, 600.0, 600.0] {
        f.record("1", err_after("http-429", 0.0));
        assert!(approx(
            f.entry("1").backoff_until.unwrap(),
            f.clock.now() + want
        ));
        f.clock.advance(want + 1.0);
    }
}

#[test]
fn non_429_retry_after_zero_does_not_take_the_saturated_edge() {
    let f = fixture();
    f.record("1", err_after("http-503", 0.0));
    assert!(approx(
        f.entry("1").backoff_until.unwrap(),
        f.clock.now() + 30.0
    ));
}

#[test]
fn hour_scale_retry_after_honored_with_margin() {
    assert_eq!(
        failure_backoff_s(1, Some(50_000.0), true),
        RETRY_AFTER_FLOOR_CAP_S
    );
    assert_eq!(failure_backoff_s(1, Some(3600.0), true), 4500.0);
    assert_eq!(failure_backoff_s(1, Some(300.0), true), 300.0);
    assert_eq!(
        failure_backoff_s(1, Some(BACKOFF_CAP_S), true),
        BACKOFF_CAP_S
    );
    assert_eq!(
        failure_backoff_s(1, Some(BACKOFF_CAP_S + 1.0), true),
        BACKOFF_CAP_S + 1.0 + 900.0
    );
    for remaining in [3600.0, 1800.0, 900.0] {
        assert!(failure_backoff_s(1, Some(remaining), true) - remaining >= 900.0);
    }
    for ask in [3601.0, 3600.0, 4000.0] {
        let want = (ask + RETRY_AFTER_MARGIN_S).min(RETRY_AFTER_FLOOR_CAP_S);
        assert_eq!(failure_backoff_s(1, Some(ask), true), want);
    }
}

#[test]
fn the_cap_sits_inside_the_trust_it_relies_on() {
    assert_eq!(RETRY_AFTER_FLOOR_CAP_S, 3600.0 + RETRY_AFTER_MARGIN_S);
    const { assert!(RETRY_AFTER_FLOOR_CAP_S <= RATE_LIMIT_TRUST_MAX_AGE_S) };
    for ask in [3600.0, 4500.0, 50_000.0, 86_400.0, f64::INFINITY] {
        assert!(failure_backoff_s(1, Some(ask), true) <= RATE_LIMIT_TRUST_MAX_AGE_S);
    }
}

#[test]
fn each_arm_is_bounded_by_the_ceiling_its_own_trust_uses() {
    for ask in [3601.0, 4500.0, 7200.0, 86_400.0, f64::INFINITY] {
        assert!(failure_backoff_s(1, Some(ask), false) <= TRUST_MAX_AGE_S);
    }
}

/// Seconds a row is un-pollable AND unknown at once after one failure at `age_at_fail`.
fn blind_window(age_at_fail: f64, ask: f64, error: &str) -> f64 {
    let f = fixture();
    let id = one("1", "a@example.com", "");
    let rec1 = |r| f.store.record(&rec("1", r), &id, None, None).unwrap();
    rec1(ok(json!({"five_hour": {"pct": 1.0}})));
    f.clock.advance(age_at_fail);
    rec1(err_after(error, ask));
    let park_end = f.store.entries(&id, &[])["1"].backoff_until.unwrap();
    let mut t = f.clock.now();
    while t < park_end {
        f.clock.set(t);
        let e = f.store.entries(&id, &[]).remove("1").unwrap();
        if e.in_backoff(t) && e.decision_value().is_none() {
            return park_end - t;
        }
        t += 1.0;
    }
    0.0
}

#[test]
fn park_bound_blind_window_equals_age_at_failure() {
    for (age, ask, want) in [
        (0.0, 5000.0, 0.0),
        (120.0, 5000.0, 120.0),
        (1800.0, 5000.0, 1800.0),
        (300.0, 3600.0, 300.0),
        (300.0, 600.0, 0.0),
    ] {
        let blind = blind_window(age, ask, "http-500");
        assert!((blind - want).abs() <= 2.0, "age {age} ask {ask}: {blind}");
    }
    for (age, ask, want) in [
        (0.0, 5000.0, 0.0),
        (2701.0, 5000.0, 1.0),
        (3600.0, 5000.0, 900.0),
    ] {
        let blind = blind_window(age, ask, "http-429");
        assert!(
            (blind - want).abs() <= 2.0,
            "[429] age {age} ask {ask}: {blind}"
        );
    }
}
