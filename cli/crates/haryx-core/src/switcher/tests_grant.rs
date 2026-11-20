//! The refresh gate, ported from upstream `tests/test_switcher.py` (`TestConsumeBackupGrant` and the
//! stash-adoption cases). The token endpoint is a local stub; nothing touches the network.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::test_support::*;
use crate::oauth::tests::stub::{Reply, Stub};
use crate::oauth::{self, extract_access_token};

const EMAIL: &str = "test@example.com";

fn creds(at: &str, rt: &str, expires_at: i64) -> String {
    json!({"claudeAiOauth": {"accessToken": at, "refreshToken": rt, "expiresAt": expires_at}})
        .to_string()
}

fn old() -> String {
    creds("sk-old", "rt-old", 1000)
}

/// A switcher with slot 1 = test@example.com, its backup `old()`, and the token endpoint stubbed.
fn gate(reply: Reply) -> (TestSwitcher, Stub, Arc<AtomicUsize>) {
    let posts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&posts);
    let stub = Stub::start(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        reply.clone()
    });
    let mut data = sample();
    data["accounts"]["1"]["email"] = json!(EMAIL);
    let mut s = TestSwitcher::linux().with_roster(data);
    s.switcher.api = stub.api();
    s.write_account_credentials("1", EMAIL, &old()).unwrap();
    (s, stub, posts)
}

fn new_tokens() -> Reply {
    Reply::json(
        200,
        &json!({"access_token": "sk-new", "refresh_token": "rt-new", "expires_in": 3600})
            .to_string(),
    )
}

fn posted_refresh_token(stub: &Stub) -> String {
    let body: Value = serde_json::from_str(&stub.requests()[0].body).unwrap();
    body["refresh_token"].as_str().unwrap().to_owned()
}

fn stored(s: &TestSwitcher) -> String {
    s.read_account_credentials("1", EMAIL, false)
}

#[test]
fn rereads_under_lock_and_posts_the_reread_bytes() {
    let (s, stub, _) = gate(new_tokens());
    s.write_account_credentials("1", EMAIL, &creds("sk-fresher", "rt-fresher", 2000))
        .unwrap();
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(posted_refresh_token(&stub), "rt-fresher");
    assert_eq!(result.error, None);
    let saved = result.credentials.unwrap();
    assert_eq!(extract_access_token(&saved).as_deref(), Some("sk-new"));
    assert_eq!(stored(&s), saved);
}

#[test]
fn leaves_a_grant_the_live_login_holds() {
    let (s, _stub, posts) = gate(new_tokens());
    s.write_credentials(&creds("sk-live", "rt-old", 1000))
        .unwrap();
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(posts.load(Ordering::SeqCst), 0);
    assert_eq!(result.error.as_deref(), Some("live-login"));
    assert!(result.credentials.is_none());
    assert_eq!(result.consumed_fp, oauth::credential_fingerprint(&old()));
    assert_eq!(stored(&s), old());
}

#[test]
fn refreshes_when_the_live_login_is_another_grant() {
    let (s, _stub, posts) = gate(new_tokens());
    s.write_credentials(&creds("sk-live", "rt-other", 1000))
        .unwrap();
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(result.error, None);
    assert_eq!(extract_access_token(&stored(&s)).as_deref(), Some("sk-new"));
}

#[test]
fn refreshes_when_there_is_no_live_login() {
    let (s, _stub, posts) = gate(new_tokens());
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(result.error, None);
}

#[test]
fn invalid_grant_returns_the_error_without_persisting() {
    let (s, _stub, _) = gate(Reply::json(400, r#"{"error": "invalid_grant"}"#));
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(result.error.as_deref(), Some("invalid_grant"));
    assert_eq!(result.consumed_fp, oauth::credential_fingerprint(&old()));
    assert_eq!(stored(&s), old());
}

#[test]
fn fresh_rotated_reread_is_adopted_without_a_second_consume() {
    let (s, _stub, posts) = gate(new_tokens());
    let rotated = creds("sk-rotated", "rt-rotated", 9_999_999_999_000);
    s.write_account_credentials("1", EMAIL, &rotated).unwrap();
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(posts.load(Ordering::SeqCst), 0);
    assert_eq!(result.credentials.as_deref(), Some(rotated.as_str()));
}

#[test]
fn removed_slot_spends_no_grant() {
    let (s, _stub, posts) = gate(new_tokens());
    s.delete_account_credentials("1", EMAIL);
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(posts.load(Ordering::SeqCst), 0);
    assert_eq!(result.error.as_deref(), Some("transient"));
}

#[test]
fn securestorage_env_refuses_consume() {
    let (mut s, _stub, posts) = gate(new_tokens());
    s.switcher.env.secure_storage_dir = Some("/elsewhere".into());
    let result = s.consume_backup_grant("1", EMAIL, &old());
    assert_eq!(result.error.as_deref(), Some("store-unmirrored"));
    assert_eq!(posts.load(Ordering::SeqCst), 0);
}

#[test]
fn cas_conflict_stashes_the_successor_and_adopts_the_racer() {
    let (s, _stub, _) = gate(new_tokens());
    let racer = creds("sk-racer", "rt-racer", 8_888_888_888_000);
    s.write_account_credentials("1", EMAIL, &racer).unwrap();
    let successor = creds("sk-new", "rt-new", 9_999_999_999_000);
    let consumed = oauth::credential_fingerprint(&old());
    let (now, reason) = s.persist_successor("1", EMAIL, &successor, consumed.as_deref());
    assert_eq!(reason, "consume-gate-cas-conflict");
    assert_eq!(now, racer);
    assert_eq!(stored(&s), racer);
    assert!(
        !s.list_unclaimed_credentials().is_empty(),
        "successor must be stashed"
    );
}

#[test]
fn next_pass_adopts_a_stashed_successor() {
    let (s, _stub, posts) = gate(new_tokens());
    let successor = creds("sk-stashed", "rt-stashed", 9_999_999_999_000);
    let mut meta = serde_json::Map::new();
    meta.insert("reason".into(), "consume-gate-persist-failed".into());
    meta.insert("configSlot".into(), "1".into());
    meta.insert(
        "consumedFp".into(),
        json!(oauth::credential_fingerprint(&old())),
    );
    meta.insert(
        "fingerprint".into(),
        json!(oauth::credential_fingerprint(&successor)),
    );
    s.store
        .write_unclaimed_credential(&successor, &meta)
        .unwrap();

    let result = s.consume_backup_grant("1", EMAIL, &old());
    // The adopted successor is fresh, so no grant is spent on top of it.
    assert_eq!(posts.load(Ordering::SeqCst), 0);
    assert_eq!(result.credentials.as_deref(), Some(successor.as_str()));
    assert_eq!(stored(&s), successor);
    assert!(s.list_unclaimed_credentials().is_empty());
}
