//! Ported from `tests/test_subscription.py` (the collector, JSON and marker cases belong to the
//! switcher/json_output/TUI ports).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;

fn plan(v: Value) -> Plan {
    v.as_object().unwrap().clone()
}

fn active_max() -> Plan {
    plan(
        json!({"tier": "default_claude_max_20x", "billingType": "stripe_subscription",
                "subscriptionStatus": "active", "planDisplayName": null}),
    )
}

fn lapsed() -> Plan {
    plan(json!({"tier": "default_claude_ai", "billingType": "none",
                "subscriptionStatus": "canceled", "planDisplayName": null}))
}

fn team() -> Plan {
    plan(
        json!({"tier": "default_claude_max_5x", "organizationType": "claude_team",
                "billingType": "stripe_subscription", "subscriptionStatus": "active",
                "planDisplayName": null}),
    )
}

fn enterprise() -> Plan {
    plan(
        json!({"tier": "default_claude_zero", "organizationType": "claude_enterprise",
                "billingType": "stripe_subscription_contracted", "subscriptionStatus": "active",
                "planDisplayName": null}),
    )
}

fn unnamed() -> Plan {
    plan(json!({"tier": "default_claude_max_5x"}))
}

fn ultra() -> Plan {
    plan(
        json!({"tier": "default_claude_max_20x", "organizationType": "claude_max",
                "planDisplayName": "Ultra"}),
    )
}

// --- fetch_plan ---------------------------------------------------------------------------------

/// Serve one canned response on an ephemeral port; returns the URL.
fn serve_once(status: u16, body: &str, headers: &[(&str, &str)]) -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let body = body.to_owned();
    let headers: Vec<tiny_http::Header> = headers
        .iter()
        .map(|(k, v)| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap())
        .collect();
    std::thread::spawn(move || {
        if let Ok(req) = server.recv() {
            let resp = headers.into_iter().fold(
                tiny_http::Response::from_string(body).with_status_code(status),
                |r, h| r.with_header(h),
            );
            let _ = req.respond(resp);
        }
    });
    format!("http://127.0.0.1:{port}/api/oauth/profile")
}

fn fetch(status: u16, body: &str, headers: &[(&str, &str)]) -> PlanFetch {
    fetch_plan_from(&serve_once(status, body, headers), "tok", FETCH_TIMEOUT)
}

#[test]
fn keeps_the_four_plan_fields_in_camel_case() {
    let payload = json!({
        "account": {"uuid": "u", "email": "e@x"},
        "organization": {"rate_limit_tier": "default_claude_max_5x",
                         "organization_type": "claude_team", "billing_type": "stripe_subscription",
                         "subscription_status": "active", "name": "not kept", "uuid": "o"},
    });
    let result = fetch(200, &payload.to_string(), &[]);
    assert_eq!(result.plan, Some(team()));
    assert_eq!(
        (
            result.account_uuid.as_str(),
            result.organization_uuid.as_str()
        ),
        ("u", "o")
    );
}

#[test]
fn keeps_the_plan_display_name() {
    let payload = json!({"organization": {"rate_limit_tier": "default_claude_max_20x",
                         "organization_type": "claude_max", "plan_display_name": "Ultra"}});
    assert_eq!(parse_plan(&payload).plan, Some(ultra()));
}

#[test]
fn no_or_empty_tier_is_no_answer() {
    let none = json!({"organization": {"billing_type": "stripe_subscription"}});
    assert!(parse_plan(&none).plan.is_none());
    assert!(
        parse_plan(&json!({"organization": {"rate_limit_tier": ""}}))
            .plan
            .is_none()
    );
    assert!(parse_plan(&json!({"organization": []})).plan.is_none());
}

#[test]
fn non_string_fields_are_dropped_not_kept() {
    let payload = json!({"organization": {"rate_limit_tier": "default_claude_ai",
                         "subscription_status": null, "billing_type": 3, "plan_display_name": ""}});
    let expected = plan(json!({"tier": "default_claude_ai", "planDisplayName": null}));
    assert_eq!(parse_plan(&payload).plan, Some(expected));
}

#[test]
fn transport_failures_are_none() {
    assert!(fetch(401, "{}", &[]).plan.is_none());
    assert!(fetch(200, "not json", &[]).plan.is_none());
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let url = format!("http://127.0.0.1:{closed}/");
    assert!(
        fetch_plan_from(&url, "tok", Duration::from_secs(2))
            .plan
            .is_none()
    );
}

#[test]
fn retry_after_is_carried_out_of_a_429() {
    let result = fetch(429, "{}", &[("Retry-After", "3600")]);
    assert!(result.plan.is_none());
    assert_eq!(result.retry_after_s, Some(3600.0));
}

// --- label and expiry ---------------------------------------------------------------------------

#[test]
fn organization_wins_over_the_seat_tier() {
    for (p, label) in [(team(), "Team"), (enterprise(), "Enterprise")] {
        assert_eq!(plan_label(Some(&p)).as_deref(), Some(label));
        let mut named = p.clone();
        named.insert("planDisplayName".into(), json!("Ultra"));
        assert_eq!(plan_label(Some(&named)).as_deref(), Some(label));
    }
}

#[test]
fn server_name_wins_unless_the_table_is_more_precise() {
    for (tier, name, label) in [
        (json!("default_claude_max_20x"), json!("Ultra"), "Ultra"),
        (json!("default_claude_max_20x"), json!(""), "Max 20x"),
        (json!("default_claude_max_20x"), Value::Null, "Max 20x"),
        (json!("default_claude_max_5x"), json!("Max"), "Max 5x"),
        (json!("default_claude_max_5x"), json!(""), "Max 5x"),
        (json!("default_claude_pro"), json!("Pro"), "Pro"),
        (Value::Null, json!("Ultra"), "Ultra"),
    ] {
        let mut p = ultra();
        p.insert("tier".into(), tier);
        p.insert("planDisplayName".into(), name);
        assert_eq!(plan_label(Some(&p)).as_deref(), Some(label));
    }
}

#[test]
fn pro_organization_wins_over_the_default_tier_but_not_a_lapse() {
    let p = plan(
        json!({"tier": "default_claude_ai", "organizationType": "claude_pro",
                        "billingType": "apple_subscription"}),
    );
    assert_eq!(plan_label(Some(&p)).as_deref(), Some("Pro"));
    let p = plan(
        json!({"tier": "default_claude_ai", "organizationType": "claude_pro",
                        "subscriptionStatus": "canceled"}),
    );
    assert!(is_expired(Some(&p)));
}

#[test]
fn tier_labels() {
    for (tier, label) in [
        ("default_claude_ai", "Free"),
        ("default_claude_max_5x", "Max 5x"),
        ("default_claude_max_20x", "Max 20x"),
        ("default_claude_pro", "default_claude_pro"),
    ] {
        assert_eq!(
            plan_label(Some(&plan(json!({"tier": tier})))).as_deref(),
            Some(label)
        );
    }
    let p = plan(json!({"tier": "default_claude_max_20x", "organizationType": "claude_max"}));
    assert_eq!(plan_label(Some(&p)).as_deref(), Some("Max 20x"));
    assert_eq!(plan_label(None), None);
}

#[test]
fn expiry() {
    assert!(!is_expired(Some(&active_max())));
    assert!(is_expired(Some(&lapsed())));
    let p = plan(json!({"tier": "default_claude_max_20x", "subscriptionStatus": "canceled"}));
    assert!(!is_expired(Some(&p)));
    let p = plan(json!({"tier": "default_claude_ai", "subscriptionStatus": "active"}));
    assert!(!is_expired(Some(&p)));
    assert!(!is_expired(None));
}

// --- records, due, tokens -----------------------------------------------------------------------

fn record(p: Plan, fetched_at: Value) -> Value {
    json!({"plan": p, "fetchedAt": fetched_at})
}

#[test]
fn due_rules() {
    assert!(is_due(None, 1000.0));
    assert!(!is_due(Some(&record(team(), json!(1000.0))), 1060.0));
    assert!(is_due(Some(&record(unnamed(), json!(1000.0))), 1060.0));
    assert!(is_due(
        Some(&record(team(), json!(1000.0))),
        1000.0 + PLAN_TTL_S
    ));
    assert!(is_due(Some(&record(team(), json!(5000.0))), 1000.0));
}

#[test]
fn malformed_record_is_invalid_and_due() {
    for r in [
        json!({"plan": team()}),
        json!({"fetchedAt": 1.0}),
        json!({"plan": {"label": "no tier"}, "fetchedAt": 1.0}),
        json!({"plan": team(), "fetchedAt": true}),
        json!("not a dict"),
    ] {
        assert!(!valid_record(&r));
        assert!(is_due(Some(&r), 1000.0));
    }
}

#[test]
fn retry_holds() {
    for p in [team(), unnamed()] {
        let r = json!({"plan": p, "fetchedAt": 0.0, "retryAt": 2000.0});
        assert!(!is_due(Some(&r), 1000.0));
    }
    let now = PLAN_TTL_S + 1000.0;
    let passed = json!({"plan": team(), "fetchedAt": 0.0, "retryAt": now - 1.0});
    assert!(is_due(Some(&passed), now));
    let skew = json!({"plan": team(), "fetchedAt": 0.0, "retryAt": 1e12});
    assert!(is_due(Some(&skew), now));
}

#[test]
fn identity_key_separates_organizations() {
    assert_ne!(
        identity_key("a@x", Some("org-1")),
        identity_key("a@x", Some("org-2"))
    );
    assert_eq!(identity_key("a@x", None), "a@x\n");
}

#[test]
fn every_recorded_uuid_must_match() {
    let fetched = PlanFetch {
        plan: Some(team()),
        account_uuid: "a".into(),
        organization_uuid: "o".into(),
        retry_after_s: None,
    };
    for (uuid, org, expected) in [
        ("a", "o", true),
        ("a", "", true),
        ("", "o", true),
        ("x", "o", false),
        ("a", "x", false),
        ("", "", false),
    ] {
        assert_eq!(matches_slot(&fetched, uuid, org), expected, "{uuid}/{org}");
    }
    let partial = PlanFetch {
        account_uuid: String::new(),
        ..fetched
    };
    assert!(!matches_slot(&partial, "a", "o"));
}

#[test]
fn retry_wait_is_floored_and_capped() {
    assert_eq!(next_retry_at(100.0, Some(0.0)), 100.0 + FAILURE_RETRY_S);
    assert_eq!(next_retry_at(100.0, None), 100.0 + FAILURE_RETRY_S);
    assert_eq!(next_retry_at(100.0, Some(3600.0)), 3700.0);
    assert_eq!(next_retry_at(100.0, Some(1e9)), 100.0 + MAX_RETRY_S);
}

fn creds(token: Option<&str>, expires_ms: Option<i64>) -> String {
    let mut blob = serde_json::Map::new();
    if let Some(t) = token {
        blob.insert("accessToken".into(), json!(t));
    }
    if let Some(e) = expires_ms {
        blob.insert("expiresAt".into(), json!(e));
    }
    json!({"claudeAiOauth": blob}).to_string()
}

#[test]
fn usable_access_token_rules() {
    let now_ms = jiff::Timestamp::now().as_millisecond();
    let future = Some(now_ms + 3_600_000);
    let past = Some(now_ms - 3_600_000);
    assert_eq!(
        usable_access_token(Some(&creds(Some("t"), future))).as_deref(),
        Some("t")
    );
    assert_eq!(usable_access_token(Some(&creds(Some("t"), past))), None);
    assert_eq!(usable_access_token(Some(&creds(None, future))), None);
    for garbage in [None, Some(""), Some("not json"), Some("{}")] {
        assert_eq!(usable_access_token(garbage), None);
    }
}

// --- store --------------------------------------------------------------------------------------

fn store_at(tmp: &TempDir, now: Option<f64>) -> PlanStore {
    let path = tmp.path().join("p.json");
    match now {
        Some(t) => PlanStore::with_clock(&path, Arc::new(move || t)),
        None => PlanStore::new(&path),
    }
}

fn save(store: &PlanStore, key: &str, update: Map<String, Value>) {
    let mut updates = Map::new();
    updates.insert(key.into(), Value::Object(update));
    store.save(&updates).unwrap();
}

#[test]
fn round_trip_and_merge_between_writers() {
    let tmp = TempDir::new().unwrap();
    let first = store_at(&tmp, None);
    save(&first, "k", success_update(team(), 1.0));
    assert_eq!(first.load()["k"], record(team(), json!(1.0)));
    let second = store_at(&tmp, None);
    save(&second, "b", success_update(lapsed(), 2.0));
    assert_eq!(first.load().len(), 2);
}

#[test]
fn older_answers_cannot_replace_newer_state() {
    let tmp = TempDir::new().unwrap();
    let store = store_at(&tmp, None);
    save(&store, "k", success_update(lapsed(), 200.0));
    save(&store, "k", success_update(active_max(), 100.0));
    assert_eq!(store.load()["k"]["plan"], Value::Object(lapsed()));

    save(&store, "j", success_update(team(), 200.0));
    save(&store, "j", failure_update(900.0, 100.0));
    assert!(store.load()["j"].get("retryAt").is_none());
}

#[test]
fn a_failure_keeps_the_plan_it_failed_to_replace() {
    let tmp = TempDir::new().unwrap();
    let store = store_at(&tmp, None);
    save(&store, "k", success_update(team(), 100.0));
    save(&store, "k", failure_update(900.0, 200.0));
    let r = &store.load()["k"];
    assert_eq!(r["plan"], Value::Object(team()));
    assert_eq!(r["fetchedAt"], json!(100.0));
    assert_eq!(r["retryAt"], json!(900.0));
}

#[test]
fn an_older_failure_cannot_shorten_a_newer_retry() {
    let tmp = TempDir::new().unwrap();
    let store = store_at(&tmp, Some(1000.0));
    save(&store, "k", failure_update(3800.0, 200.0));
    save(&store, "k", failure_update(1000.0, 100.0));
    assert_eq!(
        store.load()["k"],
        json!({"retryAt": 3800.0, "attemptedAt": 200.0})
    );
}

#[test]
fn a_success_older_than_a_failure_keeps_the_retry() {
    let tmp = TempDir::new().unwrap();
    let store = store_at(&tmp, Some(1000.0));
    save(&store, "k", failure_update(1800.0, 500.0));
    save(&store, "k", success_update(team(), 400.0));
    let r = &store.load()["k"];
    assert_eq!(r["retryAt"], json!(1800.0));
    assert_eq!(r["attemptedAt"], json!(500.0));
    assert_eq!(r["plan"], Value::Object(team()));
}

#[test]
fn future_stamps_are_skew() {
    let tmp = TempDir::new().unwrap();
    let store = store_at(&tmp, Some(1000.0));
    save(&store, "k", success_update(active_max(), 5000.0));
    save(&store, "k", success_update(lapsed(), 1000.0));
    assert_eq!(store.load()["k"], record(lapsed(), json!(1000.0)));

    save(&store, "j", success_update(active_max(), 5000.0));
    save(&store, "j", failure_update(1900.0, 1000.0));
    let r = store.load()["j"].clone();
    assert_eq!(r["plan"], Value::Object(active_max()));
    assert_eq!(r["retryAt"], json!(1900.0));
    assert!(!is_due(Some(&r), 1000.0));

    save(&store, "f", failure_update(9000.0, 5000.0));
    save(&store, "f", success_update(team(), 1000.0));
    assert_eq!(store.load()["f"], record(team(), json!(1000.0)));
}

#[test]
fn writers_cannot_interleave_a_read_merge_write() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("p.json");
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let path = path.clone();
            std::thread::spawn(move || {
                save(
                    &PlanStore::new(&path),
                    &format!("k{i}"),
                    success_update(team(), 1.0),
                );
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(PlanStore::new(&path).load().len(), 8);
}

#[test]
fn corrupt_or_missing_file_reads_as_empty() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("p.json");
    assert!(PlanStore::new(&path).load().is_empty());
    std::fs::write(&path, "{ not json").unwrap();
    assert!(PlanStore::new(&path).load().is_empty());
}

#[test]
fn snapshot_plan_defaults_to_none() {
    let snap = crate::models::AccountSnapshot::default();
    assert!(snap.subscription_plan.is_none());
    assert_eq!(snap.display_tag(), "personal");
    let named = crate::models::AccountSnapshot {
        org_name: "Acme".into(),
        subscription_plan: Some(team()),
        ..snap
    };
    assert_eq!(named.display_tag(), "Acme");
}
