//! Ported from `tests/test_oauth.py` and `tests/test_oauth_weekly_only.py`. Network tests run
//! against a local `tiny_http` stub through [`Api::with_base`], never the real endpoints.

mod network;
mod probe;
pub(crate) mod stub;

use super::*;
use jiff::tz::TimeZone;
use serde_json::json;
use std::cell::RefCell;
use stub::{Reply, Stub};

const FABLE: &str = "claude-fable-5-1";
const FABLE_CLAIM: &str = "seven_day_overage_included";

fn now_f() -> f64 {
    format::ts_to_f64(jiff::Timestamp::now())
}

fn ts(iso: &str) -> f64 {
    parse_iso_epoch(iso).unwrap()
}

fn models(m: &[&str]) -> Vec<String> {
    m.iter().map(|s| s.to_string()).collect()
}

fn creds(access: &str, refresh: Option<&str>, expires_at: i64) -> String {
    let mut oauth = json!({"accessToken": access, "expiresAt": expires_at, "scopes": ["user:inference", "user:profile"]});
    if let Some(rt) = refresh {
        oauth["refreshToken"] = json!(rt);
    }
    json!({"claudeAiOauth": oauth}).to_string()
}

fn far_future_ms() -> i64 {
    now_ms() + 86_400_000
}

fn expired_ms() -> i64 {
    now_ms() - 60_000
}

// --- extract / fingerprints / setup tokens ---

#[test]
fn extract_access_token_cases() {
    assert_eq!(
        extract_access_token(r#"{"claudeAiOauth":{"accessToken":"tok"}}"#).as_deref(),
        Some("tok")
    );
    assert_eq!(extract_access_token(r#"{"other":{}}"#), None);
    assert_eq!(extract_access_token("not json"), None);
    assert_eq!(extract_access_token(""), None);
}

#[test]
fn credential_fingerprints() {
    let a = creds("at-1", Some("rt-1"), 1);
    let b = creds("at-2", Some("rt-1"), 2);
    let c = creds("at-1", Some("rt-2"), 1);
    assert_eq!(credential_fingerprint(&a), credential_fingerprint(&b));
    assert_ne!(credential_fingerprint(&a), credential_fingerprint(&c));
    assert!(credential_fingerprint(&a).unwrap().starts_with("sha256:"));
    let key = credential_fingerprint("sk-ant-api-key").unwrap();
    assert!(key.starts_with("sha256-full:"));
    assert_ne!(key.trim_start_matches("sha256-full:"), "");
    assert_eq!(credential_fingerprint(""), None);
    assert_ne!(access_token_fingerprint(&a), access_token_fingerprint(&b));
    assert!(
        access_token_fingerprint(&a)
            .unwrap()
            .starts_with("sha256-at:")
    );
}

#[test]
fn setup_token_shape() {
    let oauth = |v: Value| v.as_object().cloned();
    assert!(is_setup_token(
        oauth(json!({"accessToken": "t", "scopes": ["user:inference"]})).as_ref()
    ));
    assert!(!is_setup_token(
        oauth(json!({"accessToken": "t", "refreshToken": "r", "scopes": ["user:inference"]}))
            .as_ref()
    ));
    assert!(!is_setup_token(
        oauth(json!({"accessToken": "t", "scopes": ["user:inference", "user:profile"]})).as_ref()
    ));
    assert!(!is_setup_token(oauth(json!({"accessToken": "t"})).as_ref()));
    assert!(!is_setup_token(None));
}

#[test]
fn login_expiry() {
    let c = json!({"claudeAiOauth": {"refreshTokenExpiresAt": 1_790_766_000_500_i64}}).to_string();
    assert_eq!(
        login_expires_at_iso(&c).as_deref(),
        Some("2026-09-30T11:00:00Z")
    );
    assert_eq!(login_expires_at_epoch(&c), Some(1_790_766_000.5));
    for bad in [
        json!(0),
        json!(-5),
        json!(true),
        json!("1790766000000"),
        json!(null),
    ] {
        let c = json!({"claudeAiOauth": {"refreshTokenExpiresAt": bad}}).to_string();
        assert_eq!(login_expires_at_iso(&c), None);
    }
    let now = 1_000_000.0;
    assert_eq!(
        login_time_left(Some(now + 8.0 * 86400.0), false, Some(now)),
        None
    );
    assert_eq!(login_time_left(Some(now + 3600.0), true, Some(now)), None);
    assert_eq!(
        login_time_left(Some(now - 5.0), false, Some(now)),
        Some(0.0)
    );
    let due = Some(now + 5.0 * 86400.0 + 12.0 * 3600.0);
    assert_eq!(
        format_login_expiry(due, false, Some(now), false).as_deref(),
        Some("re-login in 5d 12h")
    );
    assert_eq!(
        format_login_expiry(due, false, Some(now), true).as_deref(),
        Some("re-login in 5d")
    );
    assert_eq!(
        format_login_expiry(Some(now), false, Some(now), false).as_deref(),
        Some("re-login needed")
    );
}

#[test]
fn token_expiry_and_status() {
    let now = 1_800_000_000_000_i64;
    assert!(is_oauth_token_expired_at(Some(&json!(now + 60_000)), now));
    assert!(!is_oauth_token_expired_at(Some(&json!(now + 600_000)), now));
    assert!(!is_oauth_token_expired_at(Some(&json!("soon")), now));
    assert!(!is_oauth_token_expired_at(None, now));

    let at = jiff::Timestamp::from_millisecond(now).unwrap();
    let fresh = creds("t", Some("r"), now + 3_600_000);
    let status = build_token_status_at(&fresh, at, &TimeZone::UTC).unwrap();
    assert!(
        status.starts_with("oauth: fresh, refresh token yes, expires "),
        "{status}"
    );
    assert!(status.ends_with(" in 1h 0m"), "{status}");
    assert!(!status.contains("\"t\""));
    let unknown = json!({"claudeAiOauth": {"accessToken": "t"}}).to_string();
    assert_eq!(
        build_token_status_at(&unknown, at, &TimeZone::UTC).as_deref(),
        Some("oauth: unknown expiry, refresh token no")
    );
    assert_eq!(build_token_status("garbage"), None);
}

// --- format ---

#[test]
fn format_reset_grammar() {
    let now: jiff::Timestamp = "2026-03-23T12:00:00Z".parse().unwrap();
    let utc = TimeZone::UTC;
    assert_eq!(
        format_reset_in("2026-03-23T14:15:00+00:00", now, &utc).unwrap(),
        ("2h 15m".to_string(), "14:15".to_string())
    );
    assert_eq!(
        format_reset_in("2026-03-25T12:00:00+00:00", now, &utc).unwrap(),
        ("2d 0h".to_string(), "Mar 25 12:00".to_string())
    );
    assert_eq!(
        format_reset_in("2026-03-23T12:45:00Z", now, &utc)
            .unwrap()
            .0,
        "45m"
    );
    assert_eq!(
        format_reset_in("2026-03-23T11:00:00Z", now, &utc)
            .unwrap()
            .0,
        "0m"
    );
    assert_eq!(
        format_reset_in("soon", now, &utc),
        Err(TimeError::ValueError)
    );
    assert_eq!(
        format_reset_in("2026-03-23T12:45:00", now, &utc),
        Err(TimeError::TypeError)
    );
}

#[test]
fn fresh_reset_strings_fall_back_to_cached() {
    let now: jiff::Timestamp = "2026-03-23T12:00:00Z".parse().unwrap();
    let utc = TimeZone::UTC;
    let w = json!({"resets_at": "2026-03-23T13:00:00Z", "countdown": "9h", "clock": "old"});
    assert_eq!(
        fresh_reset_strings_in(&w, now, &utc),
        Some(("1h 0m".into(), "13:00".into()))
    );
    let cached = json!({"resets_at": "bad", "clock": "15:30"});
    assert_eq!(
        fresh_reset_strings_in(&cached, now, &utc),
        Some(("?".into(), "15:30".into()))
    );
    assert_eq!(fresh_reset_strings_in(&json!({"pct": 1}), now, &utc), None);
}

// --- normalisation ---

fn build(v: Value) -> Result<Option<Usage>, FetchError> {
    let now: jiff::Timestamp = "2026-03-23T12:00:00Z".parse().unwrap();
    build_usage_result_in(&v, now, &TimeZone::UTC)
}

#[test]
fn build_usage_result_windows() {
    let u = build(json!({
        "five_hour": {"utilization": 42, "resets_at": "2026-03-23T13:00:00Z"},
        "seven_day": {"utilization": 61.5, "resets_at": null},
    }))
    .unwrap()
    .unwrap();
    assert_eq!(
        u["five_hour"],
        json!({"pct": 42, "resets_at": "2026-03-23T13:00:00Z", "countdown": "1h 0m", "clock": "13:00"})
    );
    assert_eq!(u["seven_day"], json!({"pct": 61.5}));
    assert!(!u.contains_key("scoped"));
}

#[test]
fn absent_and_unknown_windows() {
    // An absent 5h window is a limit the account does not have.
    let u = build(json!({"five_hour": null, "seven_day": {"utilization": 10}}))
        .unwrap()
        .unwrap();
    assert!(!u.contains_key("five_hour"));
    let u = build(json!({"five_hour": {}, "seven_day": {"utilization": 10}}))
        .unwrap()
        .unwrap();
    assert!(!u.contains_key("five_hour"));
    // A window without a figure is unknown, not absent.
    let u = build(json!({"five_hour": {"utilization": null}, "seven_day": {"utilization": 10}}))
        .unwrap()
        .unwrap();
    assert_eq!(u["five_hour"], json!({"pct": null}));
    let only7 = Value::Object(u);
    assert_eq!(account_headroom(Some(&only7), &[]), Some(90.0));
    // Nothing to show.
    assert_eq!(build(json!({})).unwrap(), None);
    // Over-limit readings are kept as reported.
    let u = build(json!({"seven_day": {"utilization": 104.2}}))
        .unwrap()
        .unwrap();
    assert_eq!(u["seven_day"]["pct"], json!(104.2));
}

#[test]
fn an_unreadable_window_fails_the_whole_reading() {
    for broken in [json!("12"), json!(true), json!(-1), json!({})] {
        let e =
            build(json!({"five_hour": {"utilization": broken}, "seven_day": {"utilization": 1}}))
                .unwrap_err();
        assert_eq!(e.kind(), "bad-response");
    }
    let e = build(json!({"five_hour": {"resets_at": null}})).unwrap_err();
    assert_eq!(e.kind(), "bad-response");
    let e = build(json!({"seven_day": "full"})).unwrap_err();
    assert_eq!(e.kind(), "bad-response");
    let e = build(json!({"seven_day": {"utilization": 1, "resets_at": "garbage"}})).unwrap_err();
    assert_eq!(e.kind(), "ValueError");
}

#[test]
fn extra_usage_rows() {
    let base = |eu: Value| {
        build(json!({"five_hour": {"utilization": 5}, "seven_day": {"utilization": 6}, "extra_usage": eu}))
            .unwrap()
            .unwrap()
    };
    let u = base(
        json!({"is_enabled": true, "used_credits": 1250, "monthly_limit": 5000, "utilization": 25.0, "currency": "EUR"}),
    );
    assert_eq!(
        u["spend"],
        json!({"used": 12.5, "limit": 50.0, "pct": 25.0, "currency": "EUR"})
    );
    let u = base(
        json!({"is_enabled": true, "used_credits": 1250, "monthly_limit": null, "utilization": null}),
    );
    assert!(!u.contains_key("spend"));
    assert!(u.contains_key("five_hour"));
    let u = base(
        json!({"is_enabled": true, "used_credits": null, "monthly_limit": 5000, "utilization": 1}),
    );
    assert!(!u.contains_key("spend"));
    let u =
        base(json!({"is_enabled": false, "used_credits": 1, "monthly_limit": 2, "utilization": 3}));
    assert!(!u.contains_key("spend"));
    let u = base(
        json!({"is_enabled": true, "used_credits": 100, "monthly_limit": 200, "utilization": 50}),
    );
    assert_eq!(u["spend"]["currency"], json!("USD"));
}

#[test]
fn scoped_per_model_limits() {
    let u = build(json!({
        "five_hour": {"utilization": 7.0, "resets_at": null},
        "seven_day": {"utilization": 72.0, "resets_at": null},
        "seven_day_opus": null,
        "limits": [
            {"kind": "session", "percent": 7, "resets_at": null, "scope": null},
            {"kind": "weekly_all", "percent": 72, "resets_at": null, "scope": null},
            {"kind": "weekly_scoped", "percent": 100, "resets_at": "2026-03-23T15:00:00+00:00",
             "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}},
        ],
    }))
    .unwrap()
    .unwrap();
    let scoped = u["scoped"].as_array().unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0]["name"], json!("Fable"));
    assert_eq!(scoped[0]["pct"], json!(100.0));
    assert_eq!(scoped[0]["countdown"], json!("3h 0m"));
    assert!(scoped[0].get("clock").is_some());
}

// --- windows and headroom ---

#[test]
fn account_headroom_cases() {
    let h = |u: Value, m: &[&str]| account_headroom(Some(&u), &models(m));
    assert_eq!(
        h(
            json!({"five_hour": {"pct": 30}, "seven_day": {"pct": 60}}),
            &[]
        ),
        Some(40.0)
    );
    assert_eq!(
        h(
            json!({"five_hour": {"pct": 80}, "seven_day": {"pct": 60}}),
            &[]
        ),
        Some(20.0)
    );
    assert_eq!(h(json!({"seven_day": {"pct": 100}}), &[]), Some(0.0));
    assert_eq!(
        h(json!({"five_hour": {"pct": 10}, "spend": {"pct": 99}}), &[]),
        Some(90.0)
    );
    assert_eq!(h(json!({}), &[]), None);
    assert_eq!(account_headroom(None, &[]), None);
    assert_eq!(account_headroom(Some(&json!([1, 2])), &[]), None);
    assert_eq!(
        h(
            json!({"five_hour": {"pct": "50"}, "seven_day": {"pct": 20}}),
            &[]
        ),
        Some(80.0)
    );
    let scoped = json!({
        "five_hour": {"pct": 10}, "seven_day": {"pct": 20},
        "scoped": [{"name": "Fable", "pct": 100.0}, {"name": "Opus", "pct": 70.0}],
    });
    assert_eq!(account_headroom(Some(&scoped), &[]), Some(80.0));
    assert_eq!(
        account_headroom(Some(&scoped), &models(&["fable"])),
        Some(0.0)
    );
    assert_eq!(
        account_headroom(Some(&scoped), &models(&["Opus"])),
        Some(30.0)
    );
    assert_eq!(
        account_headroom(Some(&scoped), &models(&["Haiku"])),
        Some(80.0)
    );
    assert_eq!(
        account_headroom(Some(&scoped), &models(&["Opus", "Fable"])),
        Some(0.0)
    );
    assert_eq!(
        account_headroom(Some(&scoped), &models(&["all"])),
        Some(0.0)
    );
    let only = json!({"scoped": [{"name": "Fable", "pct": 40.0}]});
    assert_eq!(
        account_headroom(Some(&only), &models(&["Fable"])),
        Some(60.0)
    );
    assert_eq!(account_headroom(Some(&only), &models(&["Opus"])), None);
}

#[test]
fn relevant_windows_carry_labels_pcts_and_resets() {
    let u = json!({
        "five_hour": {"pct": 30, "resets_at": "A"},
        "seven_day": {"pct": 60.5},
        "scoped": [{"name": "Fable", "pct": 90, "resets_at": "B"}],
    });
    assert_eq!(
        relevant_windows(Some(&u), &models(&["Fable"])),
        vec![
            UsageWindow::new("5h", 30.0, Some("A")),
            UsageWindow::new("7d", 60.5, None),
            UsageWindow::new("Fable", 90.0, Some("B")),
        ]
    );
    assert_eq!(relevant_windows(Some(&u), &[]).len(), 2);
    assert!(relevant_windows(Some(&json!("x")), &[]).is_empty());
}

#[test]
fn header_reading_decisions() {
    let partial = json!({"five_hour": {"pct": 5}, "source": "headers"});
    assert_eq!(account_headroom(Some(&partial), &[]), None);
    let weekly_only = json!({"seven_day": {"pct": 30}, "source": "headers"});
    assert_eq!(account_headroom(Some(&weekly_only), &[]), Some(70.0));
    let complete = json!({"five_hour": {"pct": 5}, "seven_day": {"pct": 30}, "source": "headers"});
    assert_eq!(account_headroom(Some(&complete), &[]), Some(70.0));
    assert_eq!(account_headroom(Some(&complete), &models(&["Fable"])), None);
    assert_eq!(account_headroom(Some(&complete), &models(&["all"])), None);
    let with_fable = json!({
        "five_hour": {"pct": 5}, "seven_day": {"pct": 30}, "source": "headers",
        "scoped": [{"name": "Fable", "pct": 50}],
    });
    assert_eq!(
        account_headroom(Some(&with_fable), &models(&["fable"])),
        Some(50.0)
    );
    let rejected =
        json!({"five_hour": {"pct": 5}, "source": "headers", "rejected": {"resets_at": "R"}});
    assert_eq!(
        relevant_windows(Some(&rejected), &[]),
        vec![UsageWindow::new("rejected", 100.0, Some("R"))]
    );
    assert_eq!(account_headroom(Some(&rejected), &[]), Some(0.0));
}

// --- unified headers ---

fn unified() -> Vec<(String, String)> {
    [
        ("anthropic-ratelimit-unified-status", "allowed"),
        ("anthropic-ratelimit-unified-5h-status", "allowed"),
        ("anthropic-ratelimit-unified-5h-utilization", "0.34"),
        ("anthropic-ratelimit-unified-5h-reset", "1790766000"),
        ("anthropic-ratelimit-unified-7d-status", "allowed"),
        ("anthropic-ratelimit-unified-7d-utilization", "0.6"),
        ("anthropic-ratelimit-unified-7d-reset", "1791154800"),
        (
            "anthropic-ratelimit-unified-representative-claim",
            "five_hour",
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn with(mut h: Vec<(String, String)>, changes: &[(&str, Option<&str>)]) -> Vec<(String, String)> {
    for (k, v) in changes {
        let key = if k.starts_with("anthropic") || k.contains("After") {
            k.to_string()
        } else {
            format!("anthropic-ratelimit-unified-{k}")
        };
        h.retain(|(hk, _)| !hk.eq_ignore_ascii_case(&key));
        if let Some(v) = v {
            h.push((key, v.to_string()));
        }
    }
    h
}

fn hdrs(h: &[(String, String)]) -> Headers {
    Headers::new(h.iter().map(|(k, v)| (k.as_str(), v.clone())))
}

#[test]
fn unified_headers_map_to_the_usage_api_shape() {
    let data = parse_unified_ratelimit_headers(&hdrs(&unified()));
    assert_eq!(data["five_hour"]["utilization"], json!(34.0));
    assert_eq!(data["seven_day"]["utilization"], json!(60.0));
    assert_eq!(
        ts(data["five_hour"]["resets_at"].as_str().unwrap()),
        1_790_766_000.0
    );
    let usage = build_usage_result(&Value::Object(data)).unwrap().unwrap();
    assert_eq!(usage["five_hour"]["pct"], json!(34.0));
    assert!(usage["seven_day"].get("clock").is_some());
    assert!(!usage.contains_key("scoped"));

    let no7 = with(
        unified(),
        &[
            ("7d-status", None),
            ("7d-utilization", None),
            ("7d-reset", None),
        ],
    );
    let data = parse_unified_ratelimit_headers(&hdrs(&no7));
    assert_eq!(data.keys().collect::<Vec<_>>(), ["five_hour"]);

    let soon = with(unified(), &[("5h-reset", Some("soon"))]);
    assert_eq!(
        parse_unified_ratelimit_headers(&hdrs(&soon))["five_hour"],
        json!({"utilization": 34.0})
    );
    assert!(parse_unified_ratelimit_headers(&Headers::default()).is_empty());

    let fable = with(
        unified(),
        &[
            ("7d_oi-utilization", Some("0.82")),
            ("7d_oi-reset", Some("1791154800")),
        ],
    );
    let usage = build_usage_result(&Value::Object(parse_unified_ratelimit_headers(&hdrs(
        &fable,
    ))))
    .unwrap()
    .unwrap();
    assert_eq!(usage["scoped"][0]["name"], json!("Fable"));
    assert_eq!(usage["scoped"][0]["pct"], json!(82.0));
    // The Fable bucket header is not a 7d window header.
    assert_eq!(usage["seven_day"]["pct"], json!(60.0));

    let rejected = with(
        unified(),
        &[("status", Some("rejected")), ("reset", Some("1790766000"))],
    );
    let data = parse_unified_ratelimit_headers(&hdrs(&rejected));
    assert_eq!(
        ts(data["rejected"]["resets_at"].as_str().unwrap()),
        1_790_766_000.0
    );
}

#[test]
fn unusable_header_utilization_fails_the_reading() {
    for bad in ["abc", "-0.1", "inf", "nan"] {
        let h = with(unified(), &[("5h-utilization", Some(bad))]);
        let data = parse_unified_ratelimit_headers(&hdrs(&h));
        assert_eq!(
            build_usage_result(&Value::Object(data)).unwrap_err().kind(),
            "bad-response"
        );
    }
    let h = with(unified(), &[("5h-utilization", None)]);
    let data = parse_unified_ratelimit_headers(&hdrs(&h));
    assert!(build_usage_result(&Value::Object(data)).is_err());
}
