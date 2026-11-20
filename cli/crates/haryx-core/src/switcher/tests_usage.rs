//! Usage collection end to end against a stub usage endpoint (upstream `test_switcher.py` usage cases).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use super::test_support::*;
use crate::json_output::{
    USAGE_API_KEY, USAGE_NO_CREDENTIALS, USAGE_RELOGIN_REQUIRED, USAGE_TOKEN_EXPIRED,
};
use crate::oauth::tests::stub::{Reply, Stub};
use crate::usage_store::Decision;

const FUTURE_MS: i64 = 9_999_999_999_000;

fn creds(at: &str, expires_at: i64) -> String {
    json!({"claudeAiOauth": {"accessToken": at, "refreshToken": format!("rt-{at}"), "expiresAt": expires_at}})
        .to_string()
}

fn usage_reply(five: f64) -> Reply {
    Reply::json(
        200,
        &json!({"five_hour": {"utilization": five, "resets_at": null}, "seven_day": {"utilization": 10.0}})
            .to_string(),
    )
}

/// Two accounts; account 1 is the live login. The usage endpoint answers with `reply`.
fn setup(
    reply: impl Fn() -> Reply + Send + Sync + 'static,
) -> (TestSwitcher, Stub, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let stub = Stub::start(move |req| {
        if req.path.contains("profile") {
            return Reply::json(404, "{}");
        }
        counter.fetch_add(1, Ordering::SeqCst);
        reply()
    });
    let mut s = TestSwitcher::linux().with_roster(sample());
    s.switcher.api = stub.api();
    s.set_live("account1@example.com", "");
    (s, stub, hits)
}

fn pct(entry: &crate::usage_store::UsageEntry) -> Option<f64> {
    match entry.decision_value() {
        Some(Decision::Usage(u)) => u["five_hour"]["pct"].as_f64(),
        _ => None,
    }
}

#[test]
fn collects_both_accounts_and_marks_the_active_one() {
    let (s, _stub, hits) = setup(|| usage_reply(42.0));
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    s.write_account_credentials("2", "account2@example.com", &creds("at-2", FUTURE_MS))
        .unwrap();
    let infos = s.build_accounts_info();
    assert!(infos[0].is_active && !infos[1].is_active);
    let entries = s.collect_usage_entries(&infos, None, false);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(pct(&entries["1"]), Some(42.0));
    assert_eq!(pct(&entries["2"]), Some(42.0));
}

#[test]
fn fresh_readings_are_served_without_a_second_request() {
    let (s, _stub, hits) = setup(|| usage_reply(5.0));
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    s.write_account_credentials("2", "account2@example.com", &creds("at-2", FUTURE_MS))
        .unwrap();
    s.usage_entries_by_account(None, false);
    s.usage_entries_by_account(None, false);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        2,
        "second pass must be served from the store"
    );
}

#[test]
fn static_sentinels_need_no_network() {
    let (s, _stub, hits) = setup(|| usage_reply(1.0));
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    // Account 2 has no backup at all.
    let entries = s.usage_entries_by_account(None, false);
    assert_eq!(entries["2"].sentinel.as_deref(), Some(USAGE_NO_CREDENTIALS));
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    s.write_account_credentials("2", "account2@example.com", "sk-ant-api03-abcdef")
        .unwrap();
    let entries = s.usage_entries_by_account(None, false);
    assert_eq!(entries["2"].sentinel.as_deref(), Some(USAGE_API_KEY));
}

#[test]
fn failure_keeps_the_last_good_reading() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    let (s, _stub, _) = setup(move || {
        if c.fetch_add(1, Ordering::SeqCst) == 0 {
            usage_reply(33.0)
        } else {
            Reply::json(500, "{}")
        }
    });
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    let infos = s.build_accounts_info();
    let only_1: std::collections::HashSet<String> = ["1".to_owned()].into();
    s.collect_usage_entries(&infos, Some(&only_1), false);
    // Force the second fetch past the serve TTL by asking for it explicitly with an expired plan.
    let entries = s.collect_usage_entries(&infos, Some(&only_1), false);
    assert_eq!(
        entries["1"].last_good.as_ref().unwrap()["five_hour"]["pct"],
        33.0
    );
}

#[test]
fn expired_live_token_without_backup_reads_as_expired_and_never_refreshes() {
    let (s, _stub, hits) = setup(|| usage_reply(1.0));
    s.write_credentials(&creds("at-1", 1000)).unwrap();
    let entries = s.usage_entries_by_account(None, false);
    assert_eq!(entries["1"].sentinel.as_deref(), Some(USAGE_TOKEN_EXPIRED));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[test]
fn relogin_sentinel_after_repeated_invalid_grant() {
    let (s, _stub, _) = setup(|| Reply::json(400, r#"{"error": "invalid_grant"}"#));
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    s.write_account_credentials("2", "account2@example.com", &creds("at-2", 1000))
        .unwrap();
    let infos = s.build_accounts_info();
    let only_2: std::collections::HashSet<String> = ["2".to_owned()].into();
    let mut last = None;
    for _ in 0..crate::usage_store::AUTH_DEAD_STRIKES + 1 {
        last = Some(s.collect_usage_entries(&infos, Some(&only_2), false));
        // Each strike backs off; clear the backoff so the next pass fetches again.
        let mut rows: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(s.backup_dir.join("cache/usage.json")).unwrap(),
        )
        .unwrap();
        if let Some(row) = rows["accounts"]["2"].as_object_mut() {
            row.remove("backoffUntil");
            row.remove("nextPollAt");
            row.remove("lastAttemptAt");
        }
        std::fs::write(s.backup_dir.join("cache/usage.json"), rows.to_string()).unwrap();
    }
    let entries = last.unwrap();
    assert_eq!(
        entries["2"].sentinel.as_deref(),
        Some(USAGE_RELOGIN_REQUIRED),
        "{:?}",
        entries["2"]
    );
}

#[test]
fn snapshot_carries_policy_and_kind() {
    let (s, _stub, _) = setup(|| usage_reply(1.0));
    s.write_credentials(&creds("at-1", FUTURE_MS)).unwrap();
    let snapshot = s.accounts_snapshot(Some(&Default::default()));
    assert_eq!(snapshot.active_number.as_deref(), Some("1"));
    assert_eq!(snapshot.accounts.len(), 2);
    assert_eq!(snapshot.accounts[0].kind, "oauth");
}
