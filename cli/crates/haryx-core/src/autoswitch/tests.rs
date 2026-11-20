use std::collections::HashMap;

use serde_json::json;

use super::*;

#[test]
fn pct_label_matches_python_10g() {
    assert_eq!(pct_label(90.0), "90");
    assert_eq!(pct_label(97.5), "97.5");
    assert_eq!(pct_label(100.0 - 12.3), "87.7");
}

#[test]
fn margin_is_smaller_of_double_and_plus_three() {
    assert!(all_above_margin_met(2.0, 1.0)); // 2x of 1 = 2
    assert!(!all_above_margin_met(1.9, 1.0));
    assert!(all_above_margin_met(8.0, 5.0)); // 5+3 = 8 < 10
    assert!(!all_above_margin_met(7.9, 5.0));
}

#[test]
fn below_line_detail_formats_both_shapes() {
    assert_eq!(below_line_detail(None, 25.0, 90.0), "75% < 90%");
    let b = crate::lines::BindingWindow {
        label: "7d".into(),
        pct: 93.0,
        line: 97.0,
        resets_at: None,
        line_headroom: 4.0,
    };
    assert_eq!(below_line_detail(Some(&b), 7.0, 90.0), "7d 93% < 97%");
}

#[test]
fn threshold_map_falls_back_to_default() {
    let m = ThresholdMap::new(HashMap::from([("2".to_owned(), 80.0)]), 90.0);
    assert_eq!(m.get("2"), 80.0);
    assert_eq!(m.get("1"), 90.0);
}

#[test]
fn every_candidate_above_needs_one_measurement() {
    let c = vec!["1".to_owned(), "2".to_owned()];
    let none = HashMap::from([("1".to_owned(), None), ("2".to_owned(), None)]);
    assert!(!every_candidate_above_threshold(&c, &none));
    let mixed = HashMap::from([("1".to_owned(), Some(true)), ("2".to_owned(), None)]);
    assert!(every_candidate_above_threshold(&c, &mixed));
    assert!(!every_account_above_threshold(&c, &mixed, Some(false)));
    assert!(every_account_above_threshold(&c, &mixed, Some(true)));
}

#[test]
fn seven_day_reset_only_in_future() {
    let usage = json!({"seven_day": {"utilization": 10, "resets_at": "2030-01-01T00:00:00Z"}});
    assert!(seven_day_reset_ts(Some(&usage), 0.0).is_some());
    assert!(seven_day_reset_ts(Some(&usage), 5e9).is_none());
    assert!(seven_day_reset_ts(None, 0.0).is_none());
}

#[test]
fn account_ref_uses_integer_number() {
    assert_eq!(
        account_ref("3", "a@x.io"),
        json!({"number": 3, "email": "a@x.io"})
    );
}

#[test]
fn outcome_codes_match_exit_codes() {
    assert_eq!(TickOutcome::Switched.code(), 0);
    assert_eq!(TickOutcome::Error.code(), 1);
    assert_eq!(TickOutcome::NoAction.code(), 2);
    assert_eq!(TickOutcome::Blocked.code(), 3);
}

#[test]
fn iso_utc_matches_python_isoformat() {
    use super::tick::iso_utc;
    assert_eq!(iso_utc(1_700_000_000.0), "2023-11-14T22:13:20Z");
    assert_eq!(iso_utc(1_700_000_000.25), "2023-11-14T22:13:20.250000Z");
    // A fraction that rounds up to a whole second carries over.
    assert_eq!(iso_utc(1_700_000_000.999_999_9), "2023-11-14T22:13:21Z");
}
