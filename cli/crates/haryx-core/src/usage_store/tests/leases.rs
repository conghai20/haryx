//! Identity guard, leases, sentinels and the due pick.

use super::*;

// --- identity guard -----------------------------------------------------------------------------

#[test]
fn slot_reuse_hides_old_usage() {
    let f = fixture();
    f.record("1", ok(usage()));
    let rebound = one("1", "new@x.com", "");
    assert_eq!(f.store.entries(&rebound, &[])["1"], UsageEntry::default());
    let other_org = one("1", "a@x.com", "org-9");
    assert_eq!(f.store.entries(&other_org, &[])["1"], UsageEntry::default());
}

#[test]
fn write_replaces_mismatched_row() {
    let f = fixture();
    f.record("1", ok(usage()));
    let rebound = one("1", "new@x.com", "");
    f.store
        .record(&rec("1", err("timeout")), &rebound, None, None)
        .unwrap();
    let e = &f.store.entries(&rebound, &[])["1"];
    assert!(e.last_good.is_none());
    assert_eq!(e.consecutive_failures, 1);
}

#[test]
fn untouched_slots_survive_subset_writes() {
    let f = fixture();
    let both = BTreeMap::from([("1".into(), ok(usage())), ("2".into(), ok(usage()))]);
    f.store.record(&both, &ident(), None, None).unwrap();
    f.store
        .record(
            &rec("1", err("timeout")),
            &one("1", "a@x.com", ""),
            None,
            None,
        )
        .unwrap();
    assert_eq!(f.entry("2").last_good, Some(usage()));
}

// --- claims -------------------------------------------------------------------------------------

#[test]
fn claim_marks_in_flight() {
    let f = fixture();
    let claims = f.store.claim(&s("1"), &ident()).unwrap();
    assert_eq!(claims.keys().collect::<Vec<_>>(), ["1"]);
    assert!(f.entry("1").claimed(f.clock.now()));
    f.clock.advance(CLAIM_TTL_S + 1.0);
    assert!(!f.entry("1").claimed(f.clock.now()));
}

#[test]
fn legacy_last_attempt_claim_is_honored_during_schema_overlap() {
    let f = fixture();
    f.store.claim(&s("1"), &ident()).unwrap();
    let mut raw = f.raw();
    let row = raw["accounts"]["1"].as_object_mut().unwrap();
    row.remove("claimId");
    row.remove("claimUntil");
    f.write_raw(&raw.to_string());
    assert!(f.entry("1").claimed(f.clock.now()));
    assert!(f.reserve("1", true).is_empty());
    f.clock.advance(11.0);
    assert_eq!(f.reserve("1", true).len(), 1);
}

#[test]
fn claim_does_not_touch_measurement() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.clock.advance(100.0);
    f.store.claim(&s("1"), &ident()).unwrap();
    let e = f.entry("1");
    assert_eq!(e.last_good, Some(usage()));
    assert_eq!(e.age_s, Some(100.0));
}

#[test]
fn live_claim_outlasts_urgent_poll_interval() {
    let f = fixture();
    assert_eq!(f.reserve("1", true).len(), 1);
    f.clock.advance(61.0);
    assert!(f.reserve("1", false).is_empty());
}

#[test]
fn record_releases_claim_on_success_failure_and_sentinel() {
    let f = fixture();
    let claims = f.reserve("1", true);
    assert!(f.entry("1").claimed(f.clock.now()));
    assert_eq!(f.record_fenced("1", ok(usage()), &claims).len(), 1);
    assert!(!f.entry("1").claimed(f.clock.now()));

    let f = fixture();
    let claims = f.reserve("1", true);
    assert_eq!(f.record_fenced("1", err("timeout"), &claims).len(), 1);
    let e = f.entry("1");
    assert!(!e.claimed(f.clock.now()));
    assert_eq!(e.last_error.as_deref(), Some("timeout"));

    let f = fixture();
    let claims = f.reserve("1", true);
    let claimed_at = f.entry("1").last_attempt_at;
    f.clock.advance(1.0);
    let sentinel = FetchRecord::sentinel("token expired");
    assert_eq!(f.record_fenced("1", sentinel, &claims).len(), 1);
    let e = f.entry("1");
    assert!(!e.claimed(f.clock.now()));
    assert_eq!(e.last_attempt_at, claimed_at);
    assert!(e.sentinel.is_none());
    assert!(e.last_good.is_none());
}

#[test]
fn expired_writer_cannot_clear_or_overwrite_new_lease() {
    let f = fixture();
    let first = f.reserve("1", true);
    f.clock.advance(CLAIM_TTL_S + 1.0);
    let second = f.reserve("1", true);
    assert_ne!(first["1"], second["1"]);
    assert!(f.record_fenced("1", err("timeout"), &first).is_empty());
    let e = f.entry("1");
    assert!(e.claimed(f.clock.now()));
    assert!(e.last_error.is_none());
    assert_eq!(f.record_fenced("1", ok(usage()), &second).len(), 1);
    assert_eq!(f.entry("1").last_good, Some(usage()));
}

#[test]
fn stale_writer_cannot_replace_a_rebound_identity() {
    let f = fixture();
    let stale = f.reserve("1", true);
    let rebound = one("1", "new@x.com", "org-new");
    assert_eq!(
        f.store
            .reserve(&s("1"), &rebound, true, false)
            .unwrap()
            .len(),
        1
    );
    assert!(f.record_fenced("1", ok(usage()), &stale).is_empty());
    let e = &f.store.entries(&rebound, &[])["1"];
    assert!(e.claimed(f.clock.now()));
    assert!(e.last_good.is_none());
}

#[test]
fn mixed_record_accepts_only_the_current_claim() {
    let f = fixture();
    let nums = vec!["1".to_owned(), "2".to_owned()];
    let first = f.store.reserve(&nums, &ident(), true, false).unwrap();
    f.clock.advance(CLAIM_TTL_S + 1.0);
    let second = f.reserve("1", true);
    assert_ne!(first["1"], second["1"]);
    let outcomes = BTreeMap::from([("1".into(), err("timeout")), ("2".into(), ok(usage()))]);
    let accepted = f
        .store
        .record(&outcomes, &ident(), Some(&first), None)
        .unwrap();
    assert_eq!(accepted, BTreeSet::from(["2".to_owned()]));
    assert!(f.entry("1").last_error.is_none());
    assert!(f.entry("1").claimed(f.clock.now()));
    assert_eq!(f.entry("2").last_good, Some(usage()));
}

#[test]
fn unfenced_record_defers_to_a_live_claim_only() {
    let f = fixture();
    let claims = f.reserve("1", true);
    assert!(f.record("1", err("timeout")).is_empty());
    assert!(f.entry("1").last_error.is_none());
    assert_eq!(f.record_fenced("1", ok(usage()), &claims).len(), 1);

    let f = fixture();
    f.reserve("1", true);
    f.clock.advance(CLAIM_TTL_S + 1.0);
    assert_eq!(f.record("1", ok(usage())).len(), 1);
    assert!(!f.entry("1").claimed(f.clock.now()));
}

#[test]
fn credential_refresh_revokes_an_old_fetch_claim() {
    let f = fixture();
    let claims = f.reserve("1", true);
    f.store.clear_dead_token(&s("1"), &ident()).unwrap();
    assert!(
        f.record_fenced("1", err("invalid_grant"), &claims)
            .is_empty()
    );
    let e = f.entry("1");
    assert!(!e.claimed(f.clock.now()));
    assert!(!e.token_dead());
    assert_eq!(f.reserve("1", true).len(), 1);
}

#[test]
fn success_commits_its_new_plan() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.plan("1", (Some(f.clock.now() + 60.0), Some(60.0)));
    f.clock.advance(61.0);
    let claims = f.reserve("1", false);
    assert_eq!(claims.len(), 1);
    let next = f.clock.now() + 300.0;
    let plans = BTreeMap::from([("1".to_owned(), (Some(next), Some(300.0)))]);
    f.store
        .record(
            &rec("1", ok(usage())),
            &ident(),
            Some(&claims),
            Some(&plans),
        )
        .unwrap();
    let e = f.entry("1");
    assert_eq!(e.next_poll_at, Some(next));
    assert_eq!(e.poll_interval_s, Some(300.0));
    assert!(f.reserve("1", false).is_empty());
}

// --- sentinels, freshness, poll plan ------------------------------------------------------------

#[test]
fn sentinel_record_is_a_store_noop() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.record("1", FetchRecord::sentinel("token expired"));
    let e = f.entry("1");
    assert!(e.sentinel.is_none());
    assert_eq!(e.last_good, Some(usage()));
}

#[test]
fn refused_credential_stamp_rides_a_sentinel() {
    let f = fixture();
    let stamped =
        FetchRecord::sentinel("token expired").with_rejected_fp(Some("sha256-at:abc".into()));
    f.record("1", stamped);
    assert_eq!(
        f.entry("1").rejected_fingerprint.as_deref(),
        Some("sha256-at:abc")
    );
    f.record("1", FetchRecord::sentinel("token expired"));
    assert_eq!(
        f.entry("1").rejected_fingerprint.as_deref(),
        Some("sha256-at:abc")
    );
    f.record("1", ok(usage()));
    assert!(f.entry("1").rejected_fingerprint.is_none());
}

#[test]
fn overlay_wins_decisions_but_not_display() {
    let f = fixture();
    f.record("1", ok(usage()));
    let e = with_sentinel(f.entry("1"), Some("token expired"));
    assert_eq!(
        e.decision_value(),
        Some(Decision::Sentinel("token expired"))
    );
    assert_eq!(e.last_good, Some(usage()));
    let plain = UsageEntry {
        last_good: Some(usage()),
        ..UsageEntry::default()
    };
    assert_eq!(with_sentinel(plain.clone(), None), plain);
}

#[test]
fn fresh_within_serve_ttl() {
    let f = fixture();
    f.record("1", ok(usage()));
    let e = f.entry("1");
    let now = f.clock.now();
    assert!(e.fresh(now));
    assert!(e.fresh(now + SERVE_TTL_S));
    assert!(!e.fresh(now + SERVE_TTL_S + 1.0));
}

#[test]
fn set_and_clear_poll_plan() {
    let f = fixture();
    f.record("1", ok(usage()));
    f.plan("1", (Some(f.clock.now() + 120.0), Some(120.0)));
    let e = f.entry("1");
    assert_eq!(e.next_poll_at, Some(f.clock.now() + 120.0));
    assert_eq!(e.poll_interval_s, Some(120.0));
    assert_eq!(e.last_good, Some(usage()));
    f.plan("1", (None, None));
    let e = f.entry("1");
    assert!(e.next_poll_at.is_none() && e.poll_interval_s.is_none());
}

// --- due candidate ------------------------------------------------------------------------------

const NOW: f64 = 1_000_000.0;

fn due(entries: Vec<(&str, UsageEntry)>, cands: &[&str], now: f64) -> Option<String> {
    let map = entries
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    let cands: Vec<String> = cands.iter().map(|c| c.to_string()).collect();
    due_candidate(&cands, &map, now)
}

fn fetched(ago: f64) -> UsageEntry {
    UsageEntry {
        fetched_at: Some(NOW - ago),
        age_s: Some(ago),
        ..UsageEntry::default()
    }
}

#[test]
fn due_candidate_ordering() {
    assert_eq!(
        due(vec![("3", fetched(60.0))], &["2", "3"], NOW).as_deref(),
        Some("2")
    );
    let never = vec![("2", fetched(999.0)), ("3", UsageEntry::default())];
    assert_eq!(due(never, &["2", "3"], NOW).as_deref(), Some("3"));
    let stalest = vec![("2", fetched(60.0)), ("3", fetched(300.0))];
    assert_eq!(due(stalest, &["2", "3"], NOW).as_deref(), Some("3"));
    assert_eq!(due(vec![], &[], NOW), None);
}

#[test]
fn due_candidate_skips() {
    let sentinel = UsageEntry {
        sentinel: Some("api-key".into()),
        ..UsageEntry::default()
    };
    assert_eq!(due(vec![("2", sentinel)], &["2"], NOW), None);
    let backoff = || UsageEntry {
        backoff_until: Some(NOW + 10.0),
        ..UsageEntry::default()
    };
    assert_eq!(due(vec![("2", backoff())], &["2"], NOW), None);
    assert_eq!(
        due(vec![("2", backoff())], &["2"], NOW + 11.0).as_deref(),
        Some("2")
    );
    let planned = UsageEntry {
        next_poll_at: Some(NOW + 60.0),
        ..fetched(300.0)
    };
    assert_eq!(
        due(vec![("2", planned), ("3", fetched(60.0))], &["2", "3"], NOW).as_deref(),
        Some("3")
    );
}

#[test]
fn reset_parked_plan_is_due_for_repair_but_bounded_one_is_not() {
    let parked = |next: f64, interval: f64, last_good: Value| UsageEntry {
        last_good: Some(last_good),
        next_poll_at: Some(NOW + next),
        poll_interval_s: Some(interval),
        ..fetched(400.0)
    };
    let exhausted = json!({"seven_day": {"pct": 100.0}});
    let e = parked(86_400.0, 300.0, exhausted.clone());
    assert_eq!(due(vec![("2", e)], &["2"], NOW).as_deref(), Some("2"));
    let e = parked(600.0, 600.0, exhausted);
    assert_eq!(due(vec![("2", e)], &["2"], NOW), None);
    let scoped = json!({"five_hour": {"pct": 10.0}, "seven_day": {"pct": 10.0},
                        "scoped": [{"name": "Fable", "pct": 100.0}]});
    let e = parked(86_400.0, 300.0, scoped);
    assert_eq!(due(vec![("2", e)], &["2"], NOW).as_deref(), Some("2"));
}

#[test]
fn attempts_and_adoptions_count_as_touches() {
    let attempted = |fetched_ago: Option<f64>, attempt_ago: f64| UsageEntry {
        fetched_at: fetched_ago.map(|a| NOW - a),
        last_attempt_at: Some(NOW - attempt_ago),
        ..UsageEntry::default()
    };
    let rows = vec![
        ("2", attempted(Some(400.0), 400.0)),
        ("3", attempted(None, 60.0)),
    ];
    assert_eq!(due(rows, &["2", "3"], NOW).as_deref(), Some("2"));
    let rows = vec![
        ("2", attempted(Some(30.0), 30.0)),
        ("3", attempted(None, 60.0)),
    ];
    assert_eq!(due(rows, &["2", "3"], NOW).as_deref(), Some("3"));
    let rows = vec![
        ("2", attempted(Some(30.0), 900.0)),
        ("3", attempted(Some(600.0), 600.0)),
    ];
    assert_eq!(due(rows, &["2", "3"], NOW).as_deref(), Some("3"));
}

// --- dead token quarantine ----------------------------------------------------------------------

#[test]
fn dead_token_strikes() {
    let f = fixture();
    assert!(!f.entry("1").token_dead());
    f.record("1", err("invalid_grant"));
    assert_eq!(f.entry("1").auth_dead_strikes, 1);
    assert!(f.entry("1").token_dead());
    f.record("1", err("http-429"));
    assert_eq!(f.entry("1").auth_dead_strikes, 1);
    f.record("1", err("no_refresh_token"));
    assert_eq!(f.entry("1").auth_dead_strikes, 2);
    f.record("1", ok(usage()));
    assert_eq!(f.entry("1").auth_dead_strikes, 0);
    for _ in 0..5 {
        f.record("1", err("http-429"));
    }
    assert!(!f.entry("1").token_dead());
}

#[test]
fn quarantine_blocks_due_and_reserve_until_cleared() {
    let f = fixture();
    f.record("1", err("invalid_grant"));
    f.clock.advance(10_000.0);
    let entries = f.store.entries(&ident(), &[]);
    assert!(entries["1"].token_dead());
    assert_eq!(due_candidate(&s("1"), &entries, f.clock.now()), None);
    assert!(f.reserve("1", true).is_empty());
    assert!(f.reserve("1", false).is_empty());
    f.store.clear_dead_token(&s("1"), &ident()).unwrap();
    let e = f.entry("1");
    assert_eq!(e.auth_dead_strikes, 0);
    assert!(!e.token_dead());
    assert!(e.last_error.is_none() && e.backoff_until.is_none());
}

#[test]
fn fingerprint_bound_strikes() {
    let f = fixture();
    let struck = err("invalid_grant").with_struck_fp(Some("fp-A".into()));
    f.record("1", struck);
    let e = f.entry("1");
    assert!(e.token_dead_with(AUTH_DEAD_STRIKES, Some("fp-A")));
    assert!(!e.token_dead_with(AUTH_DEAD_STRIKES, Some("fp-B")));
    // A legacy restrike binds unconditionally, not inheriting the earlier fingerprint.
    f.record("1", err("invalid_grant"));
    let e = f.entry("1");
    assert!(e.struck_fingerprint.is_none());
    assert!(e.token_dead_with(AUTH_DEAD_STRIKES, Some("fp-anything")));
    f.store.clear_dead_token(&s("1"), &ident()).unwrap();
    assert!(f.entry("1").struck_fingerprint.is_none());
}
