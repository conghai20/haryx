//! The header probe (setup-tokens) and its per-model Fable probe.

use super::network::paths;
use super::*;

const SETUP_TOKEN: &str = "sk-ant-oat01-secret-token";

fn setup_creds() -> String {
    json!({"claudeAiOauth": {"accessToken": SETUP_TOKEN, "scopes": ["user:inference"]}}).to_string()
}

/// A stub answering the usage endpoint with a 429 and each probe by its model.
fn probe_stub(main: Reply, fable: Option<Reply>) -> Stub {
    Stub::start(move |req| {
        if req.path != "/v1/messages" {
            return Reply::json(429, "{}").header("Retry-After", "318");
        }
        let body: Value = serde_json::from_str(&req.body).unwrap_or_default();
        match body["model"].as_str() {
            Some(m) if m == USAGE_PROBE_MODEL => main.clone(),
            Some(FABLE) => fable.clone().unwrap_or_else(|| Reply::json(500, "{}")),
            _ => Reply::json(400, "{}"),
        }
    })
}

fn probe_fetch(stub: &Stub, scoped: bool, last: Option<&Value>) -> UsageOutcome {
    let c = setup_creds();
    let req = UsageFetch {
        header_probe: true,
        scoped_probe: scoped,
        last_usage: last,
        ..UsageFetch::new("3", "a@b.c", &c, false)
    };
    stub.api().try_fetch_usage_for_account(&req)
}

fn asked(stub: &Stub) -> Vec<String> {
    stub.requests()
        .iter()
        .map(|r| {
            serde_json::from_str::<Value>(&r.body).unwrap_or_default()["model"]
                .as_str()
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

fn ok_headers(h: &[(String, String)]) -> Reply {
    h.iter()
        .fold(Reply::json(200, "{}"), |r, (k, v)| r.header(k, v))
}

fn refused(h: &[(String, String)]) -> Reply {
    h.iter()
        .fold(Reply::json(429, "{}"), |r, (k, v)| r.header(k, v))
}

fn fable_headers(changes: &[(&str, Option<&str>)]) -> Vec<(String, String)> {
    let reset = (now_f() as i64 + 3 * 86400).to_string();
    let base = with(
        unified(),
        &[
            ("7d_oi-status", Some("allowed_warning")),
            ("7d_oi-utilization", Some("0.97")),
            ("7d_oi-reset", Some(&reset)),
        ],
    );
    with(base, changes)
}

#[test]
fn setup_token_skips_the_endpoint_and_probes() {
    let stub = probe_stub(ok_headers(&unified()), None);
    let out = probe_fetch(&stub, false, None);
    assert_eq!(out.error, None);
    let usage = out.usage.unwrap();
    assert_eq!(usage["five_hour"]["pct"], json!(34.0));
    assert_eq!(usage["seven_day"]["pct"], json!(60.0));
    assert_eq!(usage["source"], json!(USAGE_SOURCE_HEADERS));
    assert!(!usage.contains_key("rejected") && !usage.contains_key("scoped"));
    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1);
    let probe = &reqs[0];
    assert_eq!(
        (probe.method.as_str(), probe.path.as_str()),
        ("POST", "/v1/messages")
    );
    assert_eq!(
        probe.header("authorization"),
        Some(format!("Bearer {SETUP_TOKEN}").as_str())
    );
    assert_eq!(probe.header("anthropic-beta"), Some(OAUTH_BETA_HEADER));
    assert_eq!(probe.header("anthropic-version"), Some("2023-06-01"));
    let body: Value = serde_json::from_str(&probe.body).unwrap();
    assert_eq!(body["model"], json!(USAGE_PROBE_MODEL));
    assert_eq!(body["max_tokens"], json!(1));
    assert_eq!(body["system"], json!(USAGE_PROBE_SYSTEM));
}

#[test]
fn disabled_probe_sends_no_request() {
    let stub = probe_stub(ok_headers(&unified()), None);
    for active in [false, true] {
        let c = setup_creds();
        let out = stub
            .api()
            .try_fetch_usage_for_account(&UsageFetch::new("3", "e", &c, active));
        assert_eq!(out.error.as_deref(), Some("setup-token"));
        assert_eq!(out.usage, None);
    }
    assert!(stub.requests().is_empty());
}

#[test]
fn exhausted_and_rejected_probes_read_headers_off_the_429() {
    let h = with(
        unified(),
        &[
            ("status", Some("rejected")),
            ("5h-utilization", Some("1.0")),
        ],
    );
    let out = probe_fetch(&probe_stub(refused(&h), None), false, None);
    let usage = Value::Object(out.usage.unwrap());
    assert_eq!(usage["five_hour"]["pct"], json!(100.0));
    assert_eq!(account_headroom(Some(&usage), &[]), Some(0.0));

    let h = with(
        unified(),
        &[("status", Some("rejected")), ("Retry-After", Some("600"))],
    );
    let before = now_f();
    let out = probe_fetch(&probe_stub(refused(&h), None), false, None);
    let usage = Value::Object(out.usage.unwrap());
    assert_eq!(usage["five_hour"]["pct"], json!(34.0));
    assert_eq!(usage["rejected"]["retry_after_s"], json!(600.0));
    let until = ts(usage["rejected"]["resets_at"].as_str().unwrap());
    assert!(before + 600.0 <= until && until <= before + 660.0);
    assert_eq!(account_headroom(Some(&usage), &[]), Some(0.0));
}

#[test]
fn probe_failures_are_classified() {
    for h in [
        vec![("Retry-After".to_string(), "30".to_string())],
        with(unified(), &[("Retry-After", Some("30"))]),
    ] {
        let out = probe_fetch(&probe_stub(refused(&h), None), false, None);
        assert_eq!(out.usage, None);
        assert_eq!(out.error.as_deref(), Some("http-429"));
        assert_eq!(out.retry_after_s, Some(30.0));
    }
    let out = probe_fetch(&probe_stub(Reply::json(401, "{}"), None), false, None);
    assert_eq!(out.error.as_deref(), Some("http-401"));
    let out = probe_fetch(&probe_stub(Reply::json(200, "{}"), None), false, None);
    assert_eq!(out.error.as_deref(), Some("bad-response"));
    let c = setup_creds();
    let req = UsageFetch {
        header_probe: true,
        ..UsageFetch::new("3", "e", &c, false)
    };
    let out = Api::with_base(&stub::closed_base()).try_fetch_usage_for_account(&req);
    assert_eq!(out.error.as_deref(), Some("network"));
}

#[test]
fn a_probe_without_a_5h_header_reads_as_weekly_only() {
    let h = with(
        unified(),
        &[
            ("5h-status", None),
            ("5h-utilization", None),
            ("5h-reset", None),
        ],
    );
    let out = probe_fetch(&probe_stub(ok_headers(&h), None), false, None);
    let usage = Value::Object(out.usage.unwrap());
    assert!(usage.get("five_hour").is_none());
    assert_eq!(account_headroom(Some(&usage), &[]), Some(40.0));
}

#[test]
fn a_rejected_probe_with_an_unreadable_window_keeps_its_refusal() {
    let h = vec![
        (
            "anthropic-ratelimit-unified-5h-status".to_string(),
            "rejected".to_string(),
        ),
        (
            "anthropic-ratelimit-unified-7d-utilization".to_string(),
            "0.6".to_string(),
        ),
        (
            "anthropic-ratelimit-unified-status".to_string(),
            "rejected".to_string(),
        ),
    ];
    let out = probe_fetch(&probe_stub(refused(&h), None), false, None);
    let usage = out.usage.unwrap();
    assert!(usage.contains_key("rejected"));
    assert!(!usage.contains_key("seven_day"));
    assert_eq!(
        account_headroom(Some(&Value::Object(usage)), &[]),
        Some(0.0)
    );
}

#[test]
fn fable_bucket_is_read_from_a_fable_probe() {
    let stub = probe_stub(
        ok_headers(&unified()),
        Some(ok_headers(&fable_headers(&[]))),
    );
    let usage = probe_fetch(&stub, true, None).usage.unwrap();
    assert_eq!(asked(&stub), [USAGE_PROBE_MODEL, FABLE]);
    let fable = &usage["scoped"][0];
    assert_eq!(
        (fable["name"].clone(), fable["pct"].clone()),
        (json!("Fable"), json!(97.0))
    );
    assert!(fable.get("probe_after").is_none());
}

#[test]
fn refused_fable_reads_at_its_limit() {
    let mut h = with(
        fable_headers(&[("7d_oi-status", Some("rejected"))]),
        &[("status", Some("rejected"))],
    );
    h = with(
        h,
        &[
            ("representative-claim", Some(FABLE_CLAIM)),
            ("Retry-After", Some("600")),
        ],
    );
    let reset = h
        .iter()
        .find(|(k, _)| k.ends_with("7d_oi-reset"))
        .unwrap()
        .1
        .parse::<f64>()
        .unwrap();
    let before = now_f();
    let stub = probe_stub(ok_headers(&unified()), Some(refused(&h)));
    let usage = Value::Object(probe_fetch(&stub, true, None).usage.unwrap());
    assert!(usage.get("rejected").is_none());
    let fable = &usage["scoped"][0];
    assert_eq!(fable["pct"], json!(100.0));
    assert_eq!(ts(fable["resets_at"].as_str().unwrap()), reset);
    let after = ts(fable["probe_after"].as_str().unwrap());
    assert!(before + 600.0 <= after && after <= before + 660.0);
    assert_eq!(account_headroom(Some(&usage), &[]), Some(40.0));
    assert_eq!(
        account_headroom(Some(&usage), &models(&["Fable"])),
        Some(0.0)
    );

    // The stored spent row is kept and Fable is not asked again while paused.
    let stub2 = probe_stub(
        ok_headers(&unified()),
        Some(ok_headers(&fable_headers(&[]))),
    );
    let again = probe_fetch(&stub2, true, Some(&usage)).usage.unwrap();
    assert_eq!(asked(&stub2), [USAGE_PROBE_MODEL]);
    for key in ["name", "pct", "resets_at", "probe_after"] {
        assert_eq!(again["scoped"][0][key], usage["scoped"][0][key], "{key}");
    }

    // Once the pause ends, Fable is asked again.
    let mut stored = usage.clone();
    stored["scoped"][0]["probe_after"] = json!(iso_utc(now_f() - 1.0).unwrap());
    let stub3 = probe_stub(
        ok_headers(&unified()),
        Some(ok_headers(&fable_headers(&[(
            "7d_oi-utilization",
            Some("0.02"),
        )]))),
    );
    let fresh = probe_fetch(&stub3, true, Some(&stored)).usage.unwrap();
    assert_eq!(asked(&stub3), [USAGE_PROBE_MODEL, FABLE]);
    assert_eq!(fresh["scoped"][0]["pct"], json!(2.0));

    // Turning the probe off drops the kept row.
    let stub4 = probe_stub(ok_headers(&unified()), None);
    let off = probe_fetch(&stub4, false, Some(&usage)).usage.unwrap();
    assert!(!off.contains_key("scoped"));
}

#[test]
fn refused_fable_naming_no_deadline_is_asked_again_later() {
    let h = vec![
        (
            "anthropic-ratelimit-unified-status".to_string(),
            "rejected".to_string(),
        ),
        (
            "anthropic-ratelimit-unified-representative-claim".to_string(),
            FABLE_CLAIM.to_string(),
        ),
    ];
    let before = now_f();
    let stub = probe_stub(ok_headers(&unified()), Some(refused(&h)));
    let usage = probe_fetch(&stub, true, None).usage.unwrap();
    let fable = &usage["scoped"][0];
    assert_eq!(fable["pct"], json!(100.0));
    assert!(fable.get("resets_at").is_none());
    let after = ts(fable["probe_after"].as_str().unwrap());
    let wait = crate::poll_policy::SCOPED_PROBE_RETRY_S;
    assert!(before + wait <= after && after <= before + wait + 60.0);
}

#[test]
fn a_rejection_naming_another_limit_is_not_fables() {
    for claim in ["five_hour", "overage", "a_limit_added_later"] {
        for served in [false, true] {
            let h = with(
                fable_headers(&[
                    ("7d_oi-utilization", Some("0.4")),
                    ("7d_oi-status", Some("allowed")),
                ]),
                &[
                    ("status", Some("rejected")),
                    ("representative-claim", Some(claim)),
                    ("Retry-After", Some("600")),
                ],
            );
            let fable = if served { ok_headers(&h) } else { refused(&h) };
            let usage = probe_fetch(&probe_stub(ok_headers(&unified()), Some(fable)), true, None)
                .usage
                .unwrap();
            assert_eq!(usage["scoped"][0]["pct"], json!(40.0));
            assert!(usage["scoped"][0].get("probe_after").is_none());
            assert!(!usage.contains_key("rejected"));
        }
    }
}

#[test]
fn a_served_fable_response_can_say_the_bucket_is_spent() {
    type Changes<'a> = &'a [(&'a str, Option<&'a str>)];
    let cases: [(Changes, f64); 3] = [
        (&[("7d_oi-status", Some("rejected"))], 100.0),
        (&[("7d_oi-utilization", Some("1.0"))], 100.0),
        (&[("7d_oi-utilization", Some("1.2"))], 120.0),
    ];
    for (changes, pct) in cases {
        let h = fable_headers(changes);
        let reset = h
            .iter()
            .find(|(k, _)| k.ends_with("7d_oi-reset"))
            .unwrap()
            .1
            .parse::<f64>()
            .unwrap();
        let usage = probe_fetch(
            &probe_stub(ok_headers(&unified()), Some(ok_headers(&h))),
            true,
            None,
        )
        .usage
        .unwrap();
        let fable = &usage["scoped"][0];
        assert_eq!(fable["pct"], json!(pct));
        assert_eq!(ts(fable["probe_after"].as_str().unwrap()), reset);
        assert!(!usage.contains_key("rejected"));
    }
}

#[test]
fn no_fable_probe_after_a_refused_main_probe() {
    let main = with(
        unified(),
        &[("status", Some("rejected")), ("Retry-After", Some("600"))],
    );
    let stub = probe_stub(refused(&main), Some(ok_headers(&fable_headers(&[]))));
    let usage = probe_fetch(&stub, true, None).usage.unwrap();
    assert_eq!(usage["rejected"]["retry_after_s"], json!(600.0));
    assert!(!usage.contains_key("scoped"));
    assert_eq!(asked(&stub), [USAGE_PROBE_MODEL]);
}

#[test]
fn failed_or_bare_fable_probe_keeps_the_main_reading() {
    for fable in [
        Reply::json(404, "{}"),
        refused(&with(fable_headers(&[]), &[("Retry-After", Some("30"))])),
    ] {
        let out = probe_fetch(&probe_stub(ok_headers(&unified()), Some(fable)), true, None);
        assert_eq!(out.error, None);
        let usage = out.usage.unwrap();
        assert_eq!(usage["seven_day"]["pct"], json!(60.0));
        assert!(!usage.contains_key("scoped"));
    }
}

#[test]
fn login_credentials_never_probe() {
    let stub = probe_stub(ok_headers(&unified()), None);
    let c = creds("at", Some("rt"), far_future_ms());
    let req = UsageFetch {
        header_probe: true,
        ..UsageFetch::new("3", "e", &c, false)
    };
    let out = stub.api().try_fetch_usage_for_account(&req);
    assert_eq!(out.error.as_deref(), Some("http-429"));
    assert_eq!(paths(&stub), ["/api/oauth/usage"]);
}
