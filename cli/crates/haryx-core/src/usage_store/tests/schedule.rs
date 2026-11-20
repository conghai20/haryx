//! Reserve, 429 recency and adoption.

use super::*;

// --- reserve ------------------------------------------------------------------------------------

fn stale(f: &Fixture) {
    f.record("1", ok(usage()));
    f.clock.advance(SERVE_TTL_S + CLAIM_TTL_S + 1.0);
}

#[test]
fn reserve_wins_once_and_skips_fresh() {
    let f = fixture();
    assert_eq!(f.reserve("1", true).len(), 1);
    assert!(f.reserve("1", true).is_empty());
    assert!(f.reserve("1", false).is_empty());
    let f = fixture();
    f.record("1", ok(usage()));
    f.clock.advance(CLAIM_TTL_S + 1.0);
    assert!(f.reserve("1", true).is_empty());
}

#[test]
fn respect_plans_waits_for_next_poll() {
    let f = fixture();
    stale(&f);
    f.plan("1", (Some(f.clock.now() + 300.0), Some(300.0)));
    assert!(f.reserve("1", true).is_empty());
    f.clock.advance(301.0);
    assert_eq!(f.reserve("1", true).len(), 1);
}

#[test]
fn overslept_repair_rechecks_current_plan_under_lock() {
    let f = fixture();
    stale(&f);
    f.plan("1", (Some(f.clock.now() + 86_400.0), Some(300.0)));
    let claims = f.store.reserve(&s("1"), &ident(), true, true).unwrap();
    assert_eq!(claims.len(), 1);
    let plans = BTreeMap::from([("1".to_owned(), (Some(f.clock.now() + 300.0), Some(300.0)))]);
    f.store
        .record(
            &rec("1", ok(usage())),
            &ident(),
            Some(&claims),
            Some(&plans),
        )
        .unwrap();
    f.clock.advance(SERVE_TTL_S + 1.0);
    f.plan("1", (Some(f.clock.now() + 300.0), Some(300.0)));
    assert!(
        f.store
            .reserve(&s("1"), &ident(), false, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn scheduler_mode() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.plan("1", (Some(f.clock.now() + 60.0), Some(60.0)));
    f.clock.advance(61.0);
    assert!(f.reserve("1", true).is_empty());
    assert_eq!(f.reserve("1", false).len(), 1);

    let f = fixture();
    stale(&f);
    f.plan("1", (Some(f.clock.now() + 600.0), Some(600.0)));
    assert_eq!(f.reserve("1", false).len(), 1);
}

#[test]
fn backoff_blocks_both_modes() {
    let f = fixture();
    f.record("1", err("timeout"));
    f.clock.advance(BACKOFF_BASE_S - 1.0);
    assert!(f.reserve("1", true).is_empty());
    assert!(f.reserve("1", false).is_empty());
}

#[test]
fn unknown_row_and_identity_mismatch_win() {
    let f = fixture();
    assert_eq!(f.reserve("1", true).len(), 1);
    f.record("2", ok(usage()));
    let other = one("2", "new@x.com", "org-9");
    assert_eq!(
        f.store.reserve(&s("2"), &other, true, false).unwrap().len(),
        1
    );
}

#[test]
fn in_flight_claim_keeps_decision_trust() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.plan("1", (Some(f.clock.now() + 400.0), Some(400.0)));
    f.clock.advance(401.0);
    assert_eq!(f.reserve("1", true).len(), 1);
    let e = f.entry("1");
    assert!(e.trust_extended);
    assert_eq!(decision_usage(&e), Some(usage()));
    f.clock.advance(CLAIM_TTL_S);
    assert!(f.entry("1").decision_value().is_none());
}

// --- 429 marker and recency ---------------------------------------------------------------------

#[test]
fn last_429_marker() {
    let f = fixture();
    f.record("1", err_after("http-429", 0.0));
    let t429 = f.clock.now();
    f.clock.advance(400.0);
    f.record("1", ok(usage()));
    let e = f.entry("1");
    assert_eq!(e.consecutive_failures, 0);
    assert_eq!(e.last_429_at, Some(t429));
    let f = fixture();
    f.record("1", err("timeout"));
    assert!(f.entry("1").last_429_at.is_none());
}

#[test]
fn recent_429_is_anchored_on_the_honored_block() {
    let f = fixture();
    f.record("1", err_after("http-429", 3600.0));
    let before = f.entry("1");
    let lift = before.backoff_until.unwrap();
    assert!(before.recent_429(lift));
    assert!(!before.recent_429(lift + RECENT_429_WINDOW_S + 1.0));

    let f = fixture();
    f.record("1", err_after("http-429", 0.0));
    let before = f.entry("1");
    let lift = before.backoff_until.unwrap();
    assert!(before.recent_429(lift));
    assert!(!before.recent_429(lift + RECENT_429_WINDOW_S));
}

#[test]
fn unrelated_timeout_does_not_re_arm_recency() {
    let f = fixture();
    f.record("1", err_after("http-429", 0.0));
    f.clock.advance(400.0);
    f.record("1", ok(usage()));
    f.clock.advance(RECENT_429_WINDOW_S + 5000.0);
    assert!(!f.entry("1").recent_429(f.clock.now()));
    f.record("1", err("timeout"));
    let before = f.entry("1");
    assert!(before.last_429_at.is_some());
    assert!(!before.recent_429(before.backoff_until.unwrap()));
}

fn plan_after(before: &UsageEntry, after: &UsageEntry, recent: bool, now: f64) -> (f64, f64) {
    let input = PlanInput {
        prev_interval_s: before.poll_interval_s,
        prev_usage: before.last_good.as_ref(),
        new_usage: after.last_good.as_ref(),
        is_active: false,
        threshold: 90.0,
        models: &[],
        recent_429: recent,
        now,
        weekly_threshold: None,
    };
    poll_policy::plan_after_fetch(&input, &mut || 0.5)
}

#[test]
fn post_429_floor_engages_at_first_post_block_success() {
    let f = fixture();
    f.record("1", err_after("http-429", 3600.0));
    let before = f.entry("1");
    f.clock.set(before.backoff_until.unwrap());
    f.record("1", ok(usage()));
    let after = f.entry("1");
    let now = f.clock.now();
    let (_, interval) = plan_after(&before, &after, before.recent_429(now), now);
    assert!(before.recent_429(now));
    assert!(interval >= POST_429_MIN_INTERVAL_S);
    // The legacy recency (measured from the stamp) would have dropped the floor.
    let legacy = (now - before.last_429_at.unwrap()) < RECENT_429_WINDOW_S;
    assert!(!legacy);
    let (_, legacy_interval) = plan_after(&before, &after, legacy, now);
    assert!(legacy_interval < POST_429_MIN_INTERVAL_S);
}

#[test]
fn repeated_429_episodes_converge_to_the_wide_ceiling() {
    let f = fixture();
    let mut intervals = Vec::new();
    for _ in 0..6 {
        f.record("1", err_after("http-429", 60.0));
        let before = f.entry("1");
        f.clock.set(before.backoff_until.unwrap());
        f.record("1", ok(usage()));
        let after = f.entry("1");
        let now = f.clock.now();
        let (next, interval) = plan_after(&before, &after, before.recent_429(now), now);
        f.plan("1", (Some(next), Some(interval)));
        intervals.push(interval);
        f.clock.advance(10.0);
    }
    assert!(intervals.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*intervals.last().unwrap(), POST_429_MAX_INTERVAL_S);
    assert!(intervals[0] < intervals[2] && intervals[2] < POST_429_MAX_INTERVAL_S);
}

// --- adopt --------------------------------------------------------------------------------------

#[test]
fn adopted_reading_is_backdated_and_never_downgrades() {
    let f = fixture();
    assert_eq!(f.adopt("1", usage(), 40.0, None).len(), 1);
    let e = f.entry("1");
    assert_eq!(e.fetched_at, Some(f.clock.now() - 40.0));
    assert_eq!(e.age_s, Some(40.0));

    let f = fixture();
    let local = json!({"five_hour": {"pct": 60.0}});
    f.record("1", ok(local.clone()));
    f.clock.advance(10.0);
    assert!(f.adopt("1", usage(), 30.0, None).is_empty());
    assert_eq!(f.entry("1").last_good, Some(local));
}

#[test]
fn adopt_leaves_fetch_state_alone() {
    let f = fixture();
    f.record("1", err_after("http-429", 0.0));
    let before = f.entry("1");
    f.adopt("1", usage(), 0.0, None);
    let after = f.entry("1");
    assert_eq!(after.last_good, Some(usage()));
    assert_eq!(after.consecutive_failures, 1);
    assert_eq!(after.last_error.as_deref(), Some("http-429"));
    assert_eq!(after.backoff_until, before.backoff_until);
    assert_eq!(after.last_429_at, before.last_429_at);
}

#[test]
fn hold_keeps_every_mode_off_until_it_lapses() {
    let f = fixture();
    f.adopt("1", usage(), 0.0, Some(600.0));
    f.clock.advance(SERVE_TTL_S + 1.0);
    assert!(f.reserve("1", true).is_empty());
    assert!(f.reserve("1", false).is_empty());
    assert!(
        f.store
            .reserve(&s("1"), &ident(), false, true)
            .unwrap()
            .is_empty()
    );
    f.clock.advance(600.0);
    assert_eq!(f.reserve("1", true).len(), 1);
}

#[test]
fn held_reading_stays_decision_trusted() {
    let f = fixture();
    f.adopt("1", usage(), 0.0, Some(900.0));
    f.clock.advance(STALE_OK_S + 1.0);
    let e = f.entry("1");
    assert!(e.held(f.clock.now()));
    assert_eq!(decision_usage(&e), Some(usage()));
    f.clock.advance(900.0);
    assert!(f.entry("1").decision_value().is_none());
}

#[test]
fn hold_never_outlives_the_trust_ceiling() {
    let f = fixture();
    f.adopt("1", usage(), 3000.0, Some(3600.0));
    let e = f.entry("1");
    assert!(approx(
        e.held_until.unwrap(),
        e.fetched_at.unwrap() + TRUST_MAX_AGE_S
    ));
    f.clock.advance(TRUST_MAX_AGE_S - 3000.0);
    assert_eq!(f.reserve("1", true).len(), 1);
}

#[test]
fn latest_hold_wins_zero_lifts_none_keeps() {
    let f = fixture();
    f.adopt("1", usage(), 0.0, Some(900.0));
    f.adopt("1", usage(), 0.0, Some(60.0));
    assert_eq!(f.entry("1").held_until, Some(f.clock.now() + 60.0));
    f.adopt("1", usage(), 0.0, None);
    assert_eq!(f.entry("1").held_until, Some(f.clock.now() + 60.0));
    f.adopt("1", usage(), 0.0, Some(0.0));
    assert!(f.entry("1").held_until.is_none());
    f.clock.advance(SERVE_TTL_S + 1.0);
    assert_eq!(f.reserve("1", true).len(), 1);
}

fn rejected_reading(now: f64, deadline_in: f64, retry_after_s: Option<f64>) -> Value {
    let mut u = usage();
    u["rejected"] = json!({"retry_after_s": retry_after_s, "resets_at": iso_at(now + deadline_in)});
    u["source"] = json!(USAGE_SOURCE_HEADERS);
    u
}

#[test]
fn rejection_deadline_carries_over_as_backoff() {
    let f = fixture();
    let reading = rejected_reading(f.clock.now(), 1800.0, Some(3600.0));
    assert_eq!(f.adopt("1", reading, 1800.0, None).len(), 1);
    let e = f.entry("1");
    assert!(approx(e.backoff_until.unwrap(), f.clock.now() + 1800.0));
    assert!(e.last_error.is_none() && e.consecutive_failures == 0);
    f.clock.advance(SERVE_TTL_S + 1.0);
    assert!(f.reserve("1", false).is_empty());
    f.clock.advance(1800.0);
    assert_eq!(f.reserve("1", false).len(), 1);
}

#[test]
fn rejection_without_retry_after_installs_no_backoff() {
    let f = fixture();
    let reading = rejected_reading(f.clock.now(), 1800.0, None);
    f.adopt("1", reading.clone(), 0.0, None);
    assert!(f.entry("1").backoff_until.is_none());
    f.record("2", ok(reading));
    assert!(f.entry("2").backoff_until.is_none());
}

#[test]
fn local_rejection_with_retry_after_backs_off() {
    let f = fixture();
    f.record(
        "1",
        ok(rejected_reading(f.clock.now(), 1800.0, Some(120.0))),
    );
    assert!(approx(
        f.entry("1").backoff_until.unwrap(),
        f.clock.now() + 120.0
    ));
}

#[test]
fn past_or_unadopted_rejection_leaves_backoff_alone() {
    let f = fixture();
    f.adopt(
        "1",
        rejected_reading(f.clock.now(), -10.0, Some(3600.0)),
        0.0,
        None,
    );
    assert!(f.entry("1").backoff_until.is_none());
    f.record("2", ok(usage()));
    f.clock.advance(10.0);
    let reading = rejected_reading(f.clock.now(), 1800.0, Some(3600.0));
    assert!(f.adopt("2", reading, 30.0, None).is_empty());
    assert!(f.entry("2").backoff_until.is_none());
}

#[test]
fn rejection_deadline_never_shortens_and_is_capped() {
    let f = fixture();
    f.record("1", err_after("http-429", 4000.0));
    let parked = f.entry("1").backoff_until.unwrap();
    f.adopt(
        "1",
        rejected_reading(f.clock.now(), 1800.0, Some(3600.0)),
        0.0,
        None,
    );
    assert_eq!(f.entry("1").backoff_until, Some(parked));
    f.adopt(
        "2",
        rejected_reading(f.clock.now(), 864_000.0, Some(3600.0)),
        0.0,
        None,
    );
    let until = f.entry("2").backoff_until.unwrap();
    assert!(approx(until, f.clock.now() + RETRY_AFTER_FLOOR_CAP_S));
}

fn resetting_in(now: f64, five: f64, scoped: Option<f64>) -> Value {
    let mut u = json!({
        "five_hour": {"pct": 25.0, "resets_at": iso_at(now + five)},
        "seven_day": {"pct": 10.0, "resets_at": iso_at(now + 86_400.0)},
    });
    if let Some(sc) = scoped {
        u["scoped"] = json!([{"name": "Fable", "pct": 5.0, "resets_at": iso_at(now + sc)}]);
    }
    u
}

#[test]
fn hold_stops_at_the_readings_earliest_reset() {
    let f = fixture();
    let now = f.clock.now();
    f.adopt("1", resetting_in(now, 300.0, None), 0.0, Some(900.0));
    assert!(approx(f.entry("1").held_until.unwrap(), now + 300.0));
    f.clock.advance(300.0);
    assert_eq!(f.reserve("1", true).len(), 1);

    let f = fixture();
    f.adopt(
        "1",
        resetting_in(now, 3000.0, Some(120.0)),
        0.0,
        Some(900.0),
    );
    assert!(approx(f.entry("1").held_until.unwrap(), now + 120.0));

    let f = fixture();
    f.record("1", ok(resetting_in(now, 120.0, None)));
    assert!(
        f.adopt("1", resetting_in(now, 3000.0, None), 30.0, Some(900.0))
            .is_empty()
    );
    assert!(approx(f.entry("1").held_until.unwrap(), now + 120.0));

    let f = fixture();
    assert_eq!(
        f.adopt("1", resetting_in(now, -60.0, None), 600.0, Some(900.0))
            .len(),
        1
    );
    assert!(f.entry("1").held_until.is_none());
}

#[test]
fn due_candidate_skips_a_held_slot() {
    let f = fixture();
    let readings = BTreeMap::from([("1".into(), (usage(), 0.0)), ("2".into(), (usage(), 0.0))]);
    f.store.adopt(&readings, &ident(), Some(600.0)).unwrap();
    let plan = (Some(f.clock.now() + 60.0), Some(60.0));
    f.plan("1", plan);
    f.plan("2", plan);
    f.clock.advance(61.0);
    let cands = vec!["1".to_owned(), "2".to_owned()];
    assert_eq!(
        due_candidate(&cands, &f.store.entries(&ident(), &[]), f.clock.now()),
        None
    );
}

#[test]
fn a_claimed_fetch_still_records_over_an_adoption() {
    let f = fixture();
    let claims = f.reserve("1", true);
    f.adopt("1", usage(), 5.0, Some(600.0));
    let local = json!({"five_hour": {"pct": 61.0}});
    assert_eq!(f.record_fenced("1", ok(local.clone()), &claims).len(), 1);
    assert_eq!(f.entry("1").last_good, Some(local));
}

#[test]
fn adopt_replaces_row_of_another_account() {
    let f = fixture();
    f.store
        .record(
            &rec("2", ok(usage())),
            &one("2", "old@x.com", ""),
            None,
            None,
        )
        .unwrap();
    f.adopt("2", usage(), 0.0, None);
    let row = &f.raw()["accounts"]["2"];
    assert_eq!(row["email"], json!("b@x.com"));
    assert_eq!(row["organizationUuid"], json!("org-2"));
}
