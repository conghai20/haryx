//! The engine end to end against a stub usage endpoint: real decisions, real switches.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};

use super::{AutoSwitchEngine, Event, EventBody, TickOutcome};
use crate::oauth::tests::stub::{Reply, Stub};
use crate::settings::AutoSwitchSettings;
use crate::switcher::test_support::{TestSwitcher, creds, sample};

const FUTURE_MS: i64 = 9_999_999_999_000;

const WEEK_RESET: &str = "2099-01-05T00:00:00Z";

/// Usage per access token `at-N`: `rows[N-1]` is `(5h %, 7-day reset)`; the 7-day window is at 10%.
fn stub(rows: Vec<(f64, &'static str)>) -> Stub {
    Stub::start(move |req| {
        if req.path.contains("profile") {
            return Reply::json(404, "{}");
        }
        let auth = req.header("Authorization").unwrap_or_default();
        let (five, week_reset) = (1..=rows.len())
            .rev()
            .find(|n| auth.ends_with(&format!("at-{n}")))
            .map_or(rows[0], |n| rows[n - 1]);
        Reply::json(
            200,
            &json!({"five_hour": {"utilization": five, "resets_at": "2099-01-01T00:00:00Z"},
                    "seven_day": {"utilization": 10.0, "resets_at": week_reset}})
            .to_string(),
        )
    })
}

fn setup(five1: f64, five2: f64) -> (TestSwitcher, Stub) {
    setup_rows(vec![(five1, WEEK_RESET), (five2, WEEK_RESET)])
}

/// One account per row, account 1 live.
fn setup_rows(rows: Vec<(f64, &'static str)>) -> (TestSwitcher, Stub) {
    let n = rows.len();
    let stub = stub(rows);
    let roster = if n == 2 {
        sample()
    } else {
        let accounts: serde_json::Map<String, Value> = (1..=n)
            .map(|i| {
                let record = json!({"email": format!("account{i}@example.com"),
                    "uuid": format!("uuid-{i}"), "added": "2024-01-01T00:00:00Z"});
                (i.to_string(), record)
            })
            .collect();
        json!({"activeAccountNumber": 1, "lastUpdated": "2024-01-01T00:00:00Z",
            "sequence": (1..=n).collect::<Vec<_>>(), "accounts": accounts})
    };
    let mut s = TestSwitcher::linux().with_roster(roster);
    s.switcher.api = stub.api();
    s.set_live("account1@example.com", "");
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    for i in 1..=n {
        let (num, email, token) = (
            i.to_string(),
            format!("account{i}@example.com"),
            format!("at-{i}"),
        );
        let (num, email) = (num.as_str(), email.as_str());
        s.write_account_credentials(num, email, &creds(&token, FUTURE_MS))
            .unwrap();
        let config = json!({"oauthAccount": {"emailAddress": email, "accountUuid": format!("uuid-{num}"),
            "organizationUuid": ""}});
        s.write_account_config(num, email, &config.to_string())
            .unwrap();
    }
    (s, stub)
}

fn run_tick(
    s: &TestSwitcher,
    settings: AutoSwitchSettings,
    dry_run: bool,
) -> (TickOutcome, Vec<Event>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&events);
    let mut engine = AutoSwitchEngine::new(
        &s.switcher,
        settings,
        move |e: &Event| sink.borrow_mut().push(e.clone()),
        dry_run,
    );
    let outcome = engine.tick();
    drop(engine);
    let events = events.borrow().clone();
    (outcome, events)
}

fn last_switch(events: &[Event]) -> Option<Value> {
    events
        .iter()
        .rev()
        .find(|e| e.kind() == "switch")
        .map(Event::to_json)
}

#[test]
fn below_threshold_stays_put() {
    let (s, _stub) = setup(40.0, 5.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), false);
    assert_eq!(outcome, TickOutcome::NoAction, "{events:?}");
    let last = events.last().unwrap();
    assert!(
        matches!(&last.body, EventBody::NoSwitch { reason, detail }
        if reason == "below-threshold" && detail == "5h 40% < 90%"),
        "{last:?}"
    );
}

#[test]
fn over_threshold_switches_proactively_and_records_state() {
    let (s, _stub) = setup(95.0, 5.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), false);
    assert_eq!(outcome, TickOutcome::Switched, "{events:?}");
    let switch = last_switch(&events).unwrap();
    assert_eq!(switch["trigger"], "proactive");
    assert_eq!(
        switch["from"],
        json!({"number": 1, "email": "account1@example.com"})
    );
    assert_eq!(
        switch["to"],
        json!({"number": 2, "email": "account2@example.com"})
    );
    assert_eq!(switch["dryRun"], false);
    assert_eq!(s.current_account_number().unwrap().as_deref(), Some("2"));

    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(s.backup_dir().join(super::STATE_FILENAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(state["lastSwitchTo"], "2");
    assert_eq!(state["lastSwitchFrom"], 1);
    assert_eq!(state["leftTrigger"], "proactive");
    assert_eq!(state["leftHeadroom"], 5.0);

    // Cooldown: the next proactive pressure on account 2 does not bounce straight back.
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), false);
    assert_eq!(outcome, TickOutcome::NoAction, "{events:?}");
}

#[test]
fn dry_run_reports_without_switching() {
    let (s, _stub) = setup(95.0, 5.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Switched);
    let switch = last_switch(&events).unwrap();
    assert_eq!(switch["dryRun"], true);
    assert_eq!(s.current_account_number().unwrap().as_deref(), Some("1"));
    assert!(!s.backup_dir().join(super::STATE_FILENAME).exists());
}

#[test]
fn all_above_needs_the_margin() {
    // Everyone over the 90 line: a candidate must beat active headroom 5 by min(2x, +3) = 8.
    let (s, _stub) = setup(95.0, 93.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Blocked, "{events:?}");
    assert!(last_switch(&events).is_none());
    let (s, _stub) = setup(95.0, 92.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Switched, "{events:?}");
}

#[test]
fn at_limit_takes_any_candidate_with_room() {
    let (s, _stub) = setup(100.0, 92.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Switched, "{events:?}");
    assert_eq!(last_switch(&events).unwrap()["trigger"], "at-limit");
}

#[test]
fn all_exhausted_reports_earliest_reset() {
    let (s, _stub) = setup(100.0, 100.0);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Blocked, "{events:?}");
    let last = events.last().unwrap();
    assert!(
        matches!(&last.body, EventBody::AllExhausted { earliest_reset_at: Some(at) }
        if at == "2099-01-01T00:00:00Z"),
        "{last:?}"
    );
}

#[test]
fn best_prefers_the_sooner_weekly_reset_among_near_ties() {
    // Headroom 80 vs 75 is within the 10-point hysteresis: account 3's week resets first, so its
    // quota is the one that would be lost unused.
    let (s, _stub) = setup_rows(vec![
        (95.0, WEEK_RESET),
        (10.0, WEEK_RESET),
        (15.0, "2099-01-02T00:00:00Z"),
    ]);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Switched, "{events:?}");
    assert_eq!(last_switch(&events).unwrap()["to"]["number"], 3);
}

#[test]
fn best_keeps_most_headroom_beyond_the_hysteresis() {
    // 80 vs 65: a clear gap, so headroom still wins over the sooner reset.
    let (s, _stub) = setup_rows(vec![
        (95.0, WEEK_RESET),
        (10.0, WEEK_RESET),
        (25.0, "2099-01-02T00:00:00Z"),
    ]);
    let (outcome, events) = run_tick(&s, AutoSwitchSettings::default(), true);
    assert_eq!(outcome, TickOutcome::Switched, "{events:?}");
    assert_eq!(last_switch(&events).unwrap()["to"]["number"], 2);
}
