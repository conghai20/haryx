//! Ported from upstream `tests/test_json_output.py` (the serialisation helpers; payload tests that need
//! the switcher live with the switcher).

use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use super::*;

fn iso_in(secs: i64) -> String {
    (Timestamp::now() + SignedDuration::from_secs(secs)).to_string()
}

fn iso_at(epoch: f64, plus_secs: i64) -> String {
    (Timestamp::from_second(epoch as i64).unwrap() + SignedDuration::from_secs(plus_secs))
        .to_string()
}

fn row<'a>(f: impl FnOnce(&mut AccountRowInput<'a>)) -> Value {
    let mut input = AccountRowInput {
        number: 2,
        email: "b@example.com",
        ..Default::default()
    };
    f(&mut input);
    account_row(&input)
}

#[test]
fn usage_to_json_maps_keys_and_preserves_raw_reset() {
    let resets_at = iso_in(4 * 3600 + 30);
    let (countdown, clock) = oauth::format_reset(&resets_at).unwrap();
    let usage = json!({
        "five_hour": {"pct": 25.0, "resets_at": resets_at, "countdown": "4h", "clock": "02:00"},
        "seven_day": {"pct": 16.0},
        "spend": {"used": 12.5, "limit": 300.0, "pct": 4.0, "currency": "USD", "resets_at": resets_at},
    });
    let out = usage_to_json(&usage, None);
    assert_eq!(
        out["fiveHour"],
        json!({"pct": 25.0, "resetsAt": resets_at, "countdown": countdown, "clock": clock})
    );
    assert_eq!(out["sevenDay"], json!({"pct": 16.0}));
    assert_eq!(out["spend"]["used"], 12.5);
    assert_eq!(out["spend"]["resetsAt"], json!(resets_at));
}

#[test]
fn usage_to_json_projects_scoped_windows() {
    let resets_at = iso_in(3 * 3600 + 30);
    let (countdown, clock) = oauth::format_reset(&resets_at).unwrap();
    let usage = json!({"five_hour": {"pct": 7.0}, "scoped": [
        {"name": "Fable", "pct": 100.0, "resets_at": resets_at, "countdown": "3h", "clock": "21:59"}]});
    let out = usage_to_json(&usage, None);
    assert_eq!(
        out["scoped"],
        json!([{"pct": 100.0, "resetsAt": resets_at, "countdown": countdown, "clock": clock, "name": "Fable"}])
    );
}

#[test]
fn usage_to_json_falls_back_to_cached_strings() {
    let usage = json!({"seven_day": {"pct": 62.0, "countdown": "17h 0m", "clock": "15:59"}});
    assert_eq!(
        usage_to_json(&usage, None)["sevenDay"],
        json!({"pct": 62.0, "countdown": "17h 0m", "clock": "15:59"})
    );
    let bad = json!({"seven_day": {"pct": 62.0, "resets_at": "not-a-date", "countdown": "17h 0m", "clock": "15:59"}});
    assert_eq!(usage_to_json(&bad, None)["sevenDay"]["countdown"], "17h 0m");
}

#[test]
fn pace_fields_only_on_weekly_windows_with_fetched_at() {
    let now = 1_700_000_000.0;
    let usage = json!({
        "five_hour": {"pct": 90.0, "resets_at": iso_at(now, 4 * 3600)},
        "seven_day": {"pct": 50.0, "resets_at": iso_at(now, 6 * 86_400)},
        "scoped": [{"name": "Fable", "pct": 50.0, "resets_at": iso_at(now, 6 * 86_400)}],
    });
    let out = usage_to_json(&usage, Some(now));
    assert_eq!(out["sevenDay"]["aheadOfPace"], true);
    let expected = out["sevenDay"]["expectedPct"].as_f64().unwrap();
    assert!((expected - 14.3).abs() <= 0.1, "{expected}");
    assert!(out["sevenDay"].get("projectedExhaustionAt").is_some());
    assert_eq!(out["sevenDay"]["willLastToReset"], false);
    assert_eq!(out["scoped"][0]["aheadOfPace"], true);
    assert!(out["fiveHour"].get("aheadOfPace").is_none());
    assert!(
        usage_to_json(&usage, None)["sevenDay"]
            .get("aheadOfPace")
            .is_none()
    );

    let fresh_week =
        json!({"seven_day": {"pct": 50.0, "resets_at": iso_at(now, 7 * 86_400 - 3600)}});
    assert!(
        usage_to_json(&fresh_week, Some(now))["sevenDay"]
            .get("aheadOfPace")
            .is_none()
    );
}

#[test]
fn usage_fields_variants() {
    let usage = json!({"five_hour": {"pct": 1.0}});
    assert_eq!(
        usage_fields(Some(Decision::Usage(&usage)), None),
        ("ok", json!({"fiveHour": {"pct": 1.0}}))
    );
    let s = |x| usage_fields(Some(Decision::Sentinel(x)), None).0;
    assert_eq!(s(USAGE_NO_CREDENTIALS), "no_credentials");
    assert_eq!(s(USAGE_TOKEN_EXPIRED), "token_expired");
    assert_eq!(s(USAGE_KEYCHAIN_UNAVAILABLE), "keychain_unavailable");
    assert_eq!(s(USAGE_RELOGIN_REQUIRED), "relogin_required");
    assert_eq!(s(USAGE_FOREIGN_CREDENTIAL), "foreign_credential");
    assert_eq!(s(USAGE_API_KEY), "api_key");
    assert_eq!(usage_fields(None, None), ("unavailable", Value::Null));
}

#[test]
fn error_envelope_shape() {
    assert_eq!(
        error_envelope(&Error::Switch("boom".into())),
        json!({"schemaVersion": 1, "error": {"type": "SwitchError", "message": "boom"}})
    );
}

#[test]
fn account_row_optional_fields() {
    let base = row(|_| {});
    for key in [
        "alias",
        "group",
        "loginExpiresAt",
        "disabled",
        "threshold",
        "usageError",
        "usageRetryAt",
    ] {
        assert!(base.get(key).is_none(), "{key}");
    }
    assert_eq!(row(|r| r.alias = "dev")["alias"], "dev");
    assert_eq!(row(|r| r.group = "Client A")["group"], "Client A");
    assert_eq!(row(|r| r.disabled = true)["disabled"], true);
    assert_eq!(
        row(|r| r.login_expires_at = Some("2026-10-08T01:06:36Z"))["loginExpiresAt"],
        "2026-10-08T01:06:36Z"
    );
}

#[test]
fn unavailable_row_names_its_failure_and_retry() {
    let r = row(|r| {
        r.last_error = Some("http-429");
        r.backoff_until = Some(1_800_000_000.0);
    });
    assert_eq!(r["usageStatus"], "unavailable");
    assert_eq!(r["usageError"], "http-429");
    assert_eq!(r["usageRetryAt"], "2027-01-15T08:00:00Z");
    let lapsed = row(|r| r.last_error = Some("timeout"));
    assert_eq!(lapsed["usageError"], "timeout");
    assert!(lapsed.get("usageRetryAt").is_none());
}

#[test]
fn explained_rows_do_not_repeat_an_old_failure() {
    let usage = json!({"five_hour": {"pct": 5.0}});
    for entry in [
        Decision::Usage(&usage),
        Decision::Sentinel(USAGE_TOKEN_EXPIRED),
        Decision::Sentinel(USAGE_NO_CREDENTIALS),
    ] {
        let r = row(|r| {
            r.usage_entry = Some(entry);
            r.usage_fetched_at = Some(1_800_000_000.0);
            r.last_error = Some("http-429");
            r.backoff_until = Some(1_800_000_000.0);
        });
        assert!(r.get("usageError").is_none() && r.get("usageRetryAt").is_none());
    }
}

#[test]
fn row_key_order_matches_the_contract_fixture() {
    let usage = json!({"five_hour": {"pct": 88.0}});
    let r = row(|r| {
        r.usage_entry = Some(Decision::Usage(&usage));
        r.alias = "work";
        r.usage_fetched_at = Some(1_800_000_000.0);
        r.usage_age_s = Some(158.84);
    });
    let keys: Vec<&str> = r.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "number",
            "email",
            "organizationName",
            "organizationUuid",
            "isOrganization",
            "active",
            "usageStatus",
            "usage",
            "alias",
            "usageFetchedAt",
            "usageAgeSeconds"
        ]
    );
    assert_eq!(r["usageAgeSeconds"], 158.8);
}

#[test]
fn usage_round_trips_what_the_api_measured() {
    let internal = json!({
        "five_hour": {"pct": 12.0, "resets_at": "2099-01-01T05:00:00+00:00"},
        "seven_day": {"pct": 40.0, "resets_at": "2099-01-07T00:00:00+00:00"},
        "spend": {"used": 5.0, "limit": 50.0, "pct": 10.0, "currency": "USD",
                  "resets_at": "2099-02-01T00:00:00+00:00"},
        "scoped": [{"name": "Fable", "pct": 30.0, "resets_at": "2099-01-07T00:00:00+00:00"}],
    });
    let now = Timestamp::now().as_second() as f64;
    let mut back = usage_from_json(&usage_to_json(&internal, Some(now))).unwrap();
    for key in ["five_hour", "seven_day", "spend"] {
        let w = back[key].as_object_mut().unwrap();
        assert!(w.remove("countdown").is_some() && w.remove("clock").is_some());
    }
    let s = back["scoped"][0].as_object_mut().unwrap();
    s.remove("countdown");
    s.remove("clock");
    assert_eq!(back, internal);
}

#[test]
fn header_probe_reading_keeps_source_and_rejection() {
    let internal = json!({"five_hour": {"pct": 5.0},
        "rejected": {"retry_after_s": 60.0, "resets_at": "2099-01-01T05:00:00+00:00"},
        "source": "headers"});
    let exported = usage_to_json(&internal, None);
    assert_eq!(exported["source"], "headers");
    assert_eq!(
        exported["rejected"],
        json!({"retryAfterS": 60.0, "resetsAt": "2099-01-01T05:00:00+00:00"})
    );
    assert_eq!(usage_from_json(&exported).unwrap(), internal);
}

#[test]
fn spent_fable_row_keeps_its_probe_pause() {
    let internal = json!({"seven_day": {"pct": 40.0}, "scoped": [{"name": "Fable", "pct": 100.0,
        "resets_at": "2099-01-01T05:00:00+00:00", "probe_after": "2099-01-01T04:00:00+00:00"}],
        "source": "headers"});
    let exported = usage_to_json(&internal, None);
    assert_eq!(
        exported["scoped"][0]["probeAfter"],
        "2099-01-01T04:00:00+00:00"
    );
    let back = usage_from_json(&exported).unwrap();
    assert_eq!(
        back["scoped"][0]["probe_after"],
        "2099-01-01T04:00:00+00:00"
    );
    for bad in [json!(5), json!("soon"), json!("")] {
        let err =
            usage_from_json(&json!({"scoped": [{"name": "Fable", "pct": 100, "probeAfter": bad}]}));
        assert!(err.unwrap_err().to_string().contains("probeAfter"));
    }
}

#[test]
fn windows_without_reset_or_figure() {
    assert_eq!(
        usage_from_json(&json!({"fiveHour": {"pct": 3}})).unwrap(),
        json!({"five_hour": {"pct": 3.0}})
    );
    let internal = json!({"five_hour": {"pct": null}, "seven_day": {"pct": 40.0}});
    let exported = usage_to_json(&internal, None);
    assert!(exported["fiveHour"]["pct"].is_null());
    assert_eq!(usage_from_json(&exported).unwrap(), internal);
}

#[test]
fn malformed_usage_is_refused() {
    let cases = [
        json!(null),
        json!({}),
        json!({"fiveHour": {"pct": "12"}}),
        json!({"fiveHour": {"pct": -1}}),
        json!({"fiveHour": {"pct": true}}),
        json!({"fiveHour": {}}),
        json!({"fiveHour": {"pct": null, "resetsAt": "next tuesday"}}),
        json!({"spend": {"pct": null, "used": 1, "limit": 2, "currency": "USD"}}),
        json!({"scoped": [{"name": "Fable", "pct": null}]}),
        json!({"fiveHour": {"pct": null}, "sevenDay": {"pct": 5}, "source": "headers"}),
        json!({"sevenDay": {"pct": 1, "resetsAt": "next tuesday"}}),
        json!({"spend": {"pct": 1, "used": 1, "currency": "USD"}}),
        json!({"scoped": [{"pct": 1}]}),
        json!({"scoped": {"name": "Fable", "pct": 1}}),
        json!({"source": "headers"}),
        json!({"rejected": "yes"}),
        json!({"rejected": {"retryAfterS": "30"}}),
    ];
    for usage in cases {
        assert!(usage_from_json(&usage).is_err(), "{usage}");
    }
}

#[test]
fn plan_fields() {
    let record = json!({"plan": {"tier": "max_5x", "subscriptionStatus": "active",
        "planDisplayName": ""}, "fetchedAt": 1_800_000_000.0});
    let out = subscription_plan_fields(&record);
    assert_eq!(out["tier"], "max_5x");
    assert_eq!(out["subscriptionStatus"], "active");
    assert!(out.get("planDisplayName").is_none());
    assert_eq!(out["expired"], false);
    assert_eq!(out["fetchedAt"], "2027-01-15T08:00:00Z");
}
