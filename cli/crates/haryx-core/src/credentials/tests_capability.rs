//! Keychain capability verdicts, pins and the pin lift (upstream `tests/test_macos_keychain_contract.py`,
//! store-level cases).

use super::test_support::*;
use super::*;

const SLOT_EMAIL: &str = "c@e.com";

/// The OAuth write falls back to the file because `set` fails; `delete_ok` decides the residual verdict.
fn pin_through_a_failed_write(sb: &Sandbox, store: &CredentialStore, delete_ok: bool) {
    sb.keychain.fail_when(move |op, _, _| match op {
        Op::Set => true,
        Op::Delete => !delete_ok,
        Op::Get => false,
    });
    store
        .write_oauth_credentials("written-by-the-fallback")
        .unwrap();
    sb.keychain.heal();
}

// ---- our own file mode is not a Keychain failure --------------------------------------------------------

#[test]
fn an_empty_backup_stays_empty_after_our_own_file_mode_write() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.pin_file_mode(true);
    assert_eq!(
        store.read_account_credentials_ex("9", "x@e.com", false),
        (String::new(), false)
    );
}

#[test]
fn a_write_fallback_does_not_certify_an_unread_backup() {
    let sb = Sandbox::macos();
    let store = sb.store();
    sb.keychain.fail_all();
    assert_eq!(
        store.read_account_credentials_ex("3", SLOT_EMAIL, false),
        (String::new(), true)
    );

    store.write_oauth_credentials(&creds("fb")).unwrap();
    assert!(store.file_mode_is_ours(), "premise: the write pinned");

    let (_value, unreadable) = store.read_account_credentials_ex("3", SLOT_EMAIL, false);
    assert!(
        unreadable,
        "a write fallback for the ACTIVE credential does not certify a backup"
    );
}

#[test]
fn a_write_fallback_does_not_certify_the_active_read_either() {
    let sb = Sandbox::macos();
    let store = sb.store();
    sb.keychain.fail_all();
    assert!(store.read_active_credentials().degraded, "premise");
    store.write_oauth_credentials(&creds("x")).unwrap();
    assert!(store.file_mode_is_ours(), "premise: the write pinned");
    assert!(store.read_active_credentials().degraded);
}

#[test]
fn a_real_keychain_failure_still_reports_unreadable() {
    let sb = Sandbox::macos();
    sb.keychain.fail_when(|op, _, _| op == Op::Get);
    let (_v, unreadable) = sb
        .store()
        .read_account_credentials_ex("9", "x@e.com", false);
    assert!(unreadable);
}

#[test]
fn a_stale_failure_flag_does_not_condemn_a_working_read() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.set_usable_cache(Some(false)); // an earlier op failed
    assert_eq!(
        store.read_account_credentials_ex("9", "x@e.com", false),
        (String::new(), false)
    );
}

#[test]
fn a_recovered_keychain_is_not_condemned_by_the_previous_read() {
    let mut sb = Sandbox::macos();
    sb.tunables.keychain_recheck_cooldown = Duration::ZERO;
    let store = sb.store();
    sb.keychain.fail_all();
    assert_eq!(
        store.read_account_credentials_ex("9", "x@e.com", false),
        (String::new(), true)
    );
    sb.keychain.heal();
    assert_eq!(
        store.read_account_credentials_ex("9", "x@e.com", false),
        (String::new(), false)
    );
}

fn fail_one_call_then_recover(sb: &Sandbox, store: &CredentialStore) {
    sb.keychain.fail_all();
    assert!(store.kc_call(|kc| kc.get_password("svc", "acct")).is_err());
    assert!(store.keychain_unreadable(), "premise: inside the cooldown");
    sb.keychain.heal();
    store.expire_cooldown();
    assert!(!store.keychain_unreadable(), "premise: cooldown lapsed");
    store.kc_call(|kc| kc.get_password("svc", "acct")).unwrap();
}

#[test]
fn a_recovered_keychain_does_not_latch_through_a_later_pin() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.set_usable_cache(Some(true));
    fail_one_call_then_recover(&sb, &store);
    store.pin_file_mode(true);
    assert!(!store.keychain_unreadable());
}

#[test]
fn a_recovered_keychain_does_not_latch_through_an_unverified_pin() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.set_usable_cache(Some(true));
    fail_one_call_then_recover(&sb, &store);
    store.pin_file_mode(false);
    assert!(!store.keychain_unreadable());
}

#[test]
fn an_idle_backup_read_does_not_erase_the_active_verdict() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.set_usable_cache(Some(true));
    sb.keychain
        .fail_when(|op, service, _| op == Op::Get && service.contains("credentials"));
    assert!(store.read_active_credentials().degraded, "premise");
    store.read_account_credentials("3", "c@example.com", false);
    assert!(store.read_active_credentials().degraded);
}

#[test]
fn the_active_verdict_survives_a_pin_only_when_it_is_true() {
    let sb = Sandbox::macos();
    let store = sb.store();
    // 1. nothing ever failed
    store.set_usable_cache(Some(true));
    store.read_active_credentials();
    store.pin_file_mode(true);
    assert!(!store.active_read_failed());
    assert!(!store.read_active_credentials().degraded);

    // 2. a failure the cooldown healed
    let sb = Sandbox::macos();
    let store = sb.store();
    store.set_usable_cache(Some(true));
    sb.keychain.fail_when(|op, _, _| op == Op::Get);
    store.read_active_credentials();
    assert!(store.active_read_failed(), "premise: it failed");
    sb.keychain.heal();
    store.expire_cooldown();
    store.read_active_credentials();
    assert!(!store.active_read_failed(), "premise: healed");
    store.pin_file_mode(true);
    assert!(!store.active_read_failed());
    assert!(!store.read_active_credentials().degraded);
}

#[test]
fn two_concurrent_backup_reads_keep_their_own_verdicts() {
    let mut sb = Sandbox::macos();
    sb.tunables.keychain_recheck_cooldown = Duration::ZERO;
    *sb.keychain.delay.lock().unwrap() = Duration::from_millis(2);
    sb.keychain
        .fail_when(|op, _, account| op == Op::Get && account.contains("account-2-"));
    let store = Arc::new(sb.store());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sibling = {
        let store = store.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                store.read_account_credentials_ex("9", "i@e.com", false);
            }
        })
    };
    let lost = (0..40)
        .filter(|_| !store.read_account_credentials_ex("2", "b@e.com", false).1)
        .count();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    sibling.join().unwrap();
    assert_eq!(
        lost, 0,
        "reads lost their own unreadable verdict to a concurrent read"
    );
}

#[test]
fn a_verified_clear_ends_the_degraded_verdict() {
    let sb = Sandbox::macos();
    let store = sb.store();
    sb.keychain.fail_when(|op, _, _| op == Op::Get);
    assert!(store.read_active_credentials().degraded, "premise");
    sb.keychain.heal();
    let cleared = store.delete_active_keychain_entry();
    store.pin_file_mode(cleared);
    assert!(!store.read_active_credentials().degraded);
}

#[test]
fn a_verified_clear_does_not_mask_a_later_failure() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.pin_file_mode(store.delete_active_keychain_entry());
    // A later backup read fails and re-arms the deadline; the cooldown lapses; the ACTIVE read fails.
    sb.keychain.fail_when(|op, _, _| op == Op::Get);
    store.read_account_credentials("9", "i@e.com", false);
    store.expire_cooldown();
    assert!(
        store.read_active_credentials().degraded,
        "premise: the active read failed"
    );
    assert!(store.read_active_credentials().degraded);
}

#[test]
fn a_failed_clear_survives_an_unrelated_success() {
    let sb = Sandbox::macos();
    let store = sb.store();
    sb.keychain.fail_when(|op, _, _| op != Op::Get);
    assert!(
        store
            .kc_call(|kc| kc.set_password("svc", "acct", "v"))
            .is_err()
    );
    let cleared = store.delete_active_keychain_entry();
    assert!(!cleared);
    store.pin_file_mode(cleared);

    sb.keychain.heal();
    sb.keychain
        .put_for(HARYX_BACKUP_SERVICE, "account-9-i@e.com", "sibling-token");
    assert_eq!(
        store.read_account_credentials("9", "i@e.com", false),
        "sibling-token"
    );
    assert!(store.read_active_credentials().degraded);
}

#[test]
fn a_lapsed_cooldown_clears_the_unreadable_verdict() {
    let sb = Sandbox::macos();
    let store = sb.store();
    sb.keychain.fail_all();
    assert!(store.kc_call(|kc| kc.get_password("svc", "acct")).is_err());
    assert!(store.keychain_unreadable());
    store.expire_cooldown();
    assert!(!store.keychain_unreadable());
}

#[test]
fn off_macos_there_is_no_keychain_to_be_unreadable() {
    let sb = Sandbox::linux();
    let store = sb.store();
    store.set_usable_cache(Some(false));
    assert!(!store.keychain_unreadable());
    assert!(!store.use_keychain());
    assert!(store.uses_file_backup_backend());
}

#[test]
fn a_missing_slot_is_absent_not_unreadable_after_our_own_pin() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.pin_file_mode(true);
    let active = store.read_active_credentials();
    assert_eq!(active.value.as_deref(), Some(""));
    assert!(!active.keychain_unavailable);
    assert_eq!(
        store.read_account_credentials_ex("9", "x@e.com", false),
        (String::new(), false)
    );
}

#[test]
fn a_transient_read_failure_is_retried_once() {
    let sb = Sandbox::macos();
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("live"));
    let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = failures.clone();
    sb.keychain.fail_when(move |op, _, _| {
        op == Op::Get && counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
    });
    let active = sb.store().read_active_credentials();
    assert_eq!(active.value, Some(creds("live")));
    assert!(!active.degraded);
    assert_eq!(sb.keychain.calls_for(Op::Get, ACCOUNT), 2);
}

// ---- a pinned file mode ends when its file is gone ------------------------------------------------------

#[test]
fn the_keychain_is_read_again_once_our_file_is_gone() {
    for delete_ok in [true, false] {
        let sb = Sandbox::macos();
        let store = sb.store();
        pin_through_a_failed_write(&sb, &store, delete_ok);
        std::fs::remove_file(sb.default_cred_file()).unwrap();
        sb.keychain
            .put(CLAUDE_CODE_KEYCHAIN_SERVICE, "rotated-by-claude");
        let active = store.read_active_credentials();
        assert_eq!(
            active.value.as_deref(),
            Some("rotated-by-claude"),
            "delete_ok={delete_ok}"
        );
    }
}

#[test]
fn the_pin_holds_while_our_file_is_there() {
    for delete_ok in [true, false] {
        let sb = Sandbox::macos();
        let store = sb.store();
        pin_through_a_failed_write(&sb, &store, delete_ok);
        sb.keychain
            .put(CLAUDE_CODE_KEYCHAIN_SERVICE, "stale-residual");
        let active = store.read_active_credentials();
        assert_eq!(active.value.as_deref(), Some("written-by-the-fallback"));
        assert!(!store.use_keychain());
    }
}

#[test]
fn a_still_unreadable_keychain_reads_as_unavailable_not_empty() {
    let sb = Sandbox::macos();
    let store = sb.store();
    pin_through_a_failed_write(&sb, &store, false);
    std::fs::remove_file(sb.default_cred_file()).unwrap();
    sb.keychain.fail_all();
    let active = store.read_active_credentials();
    assert_eq!(active.value.as_deref(), Some(""));
    assert!(active.keychain_unavailable);
    assert!(active.degraded);
}

#[test]
fn an_empty_file_lifts_the_pin_like_a_missing_one() {
    let sb = Sandbox::macos();
    let store = sb.store();
    pin_through_a_failed_write(&sb, &store, true);
    std::fs::write(sb.default_cred_file(), "").unwrap();
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, "rotated-by-claude");
    assert_eq!(
        store.read_credentials().as_deref(),
        Some("rotated-by-claude")
    );
}

#[cfg(unix)]
#[test]
fn an_unstattable_file_keeps_the_pin() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::macos();
    let store = sb.store();
    pin_through_a_failed_write(&sb, &store, true);
    let dir = sb.home().join(".claude");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    let stattable = std::fs::metadata(sb.default_cred_file()).is_ok();
    store.lift_pin_if_oauth_file_gone();
    let still_pinned = !store.use_keychain();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !stattable {
        assert!(
            still_pinned,
            "a file that cannot be stat'ed must keep the pin"
        );
    }
}

#[test]
fn a_managed_key_repin_is_not_lifted_by_a_stale_oauth_flag() {
    let sb = Sandbox::macos();
    let store = sb.store();
    pin_through_a_failed_write(&sb, &store, true);
    store
        .write_managed_credentials("sk-ant-api03-ffffffffffffffffffffffffffffffff")
        .unwrap();
    sb.keychain.put(
        CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE,
        "sk-ant-api03-ssssssssssssssssssssssssssssssss",
    );
    store.read_active_credentials();
    assert!(
        !store.use_keychain(),
        "the managed-key pin was lifted by the OAuth pin's leftover flag"
    );
}

#[test]
fn our_own_oauth_clear_does_not_lift_the_pin() {
    let sb = Sandbox::macos();
    let store = sb.store();
    pin_through_a_failed_write(&sb, &store, false);
    store.clear_oauth_credential();
    sb.keychain.put(CLAUDE_CODE_KEYCHAIN_SERVICE, "residual");
    store.read_active_credentials();
    assert!(!store.use_keychain());
}

#[test]
fn any_later_pin_disarms_the_lift() {
    let sb = Sandbox::macos();
    let store = sb.store();
    pin_through_a_failed_write(&sb, &store, true);
    store.pin_file_mode(true);
    std::fs::remove_file(sb.default_cred_file()).unwrap();
    sb.keychain.put(CLAUDE_CODE_KEYCHAIN_SERVICE, "residual");
    store.read_active_credentials();
    assert!(!store.use_keychain());
}

// ---- the real `security` backend through a fake binary --------------------------------------------------

#[cfg(unix)]
fn fake_security(dir: &Path, body: &str) -> crate::keychain::Security {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("security");
    std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    crate::keychain::Security::with_binary(bin)
}

#[cfg(unix)]
#[test]
fn a_locked_security_binary_reads_as_keychain_unavailable() {
    let sb = Sandbox::macos();
    let security = fake_security(&sb.home(), "echo 'locked' >&2; exit 36");
    let store = CredentialStore::with_keychain(sb.host(), Arc::new(security), sb.tunables);
    let active = store.read_active_credentials();
    assert_eq!(active.value.as_deref(), Some(""));
    assert!(active.keychain_unavailable && active.degraded);
    assert_eq!(
        store.read_account_credentials_ex("1", "a@b.c", false),
        (String::new(), true)
    );
}

#[cfg(unix)]
#[test]
fn an_absent_item_from_security_is_an_empty_slot() {
    let sb = Sandbox::macos();
    let security = fake_security(&sb.home(), "exit 44");
    let store = CredentialStore::with_keychain(sb.host(), Arc::new(security), sb.tunables);
    let active = store.read_active_credentials();
    assert_eq!(active.value.as_deref(), Some(""));
    assert!(!active.keychain_unavailable && !active.degraded);
    assert_eq!(
        store.read_account_credentials_ex("1", "a@b.c", false),
        (String::new(), false)
    );
}

#[cfg(unix)]
#[test]
fn security_is_asked_for_the_haryx_backup_service() {
    let sb = Sandbox::macos();
    let log = sb.home().join("argv.log");
    let security = fake_security(
        &sb.home(),
        &format!(
            "echo \"$@\" >> '{}'; echo backup-value; exit 0",
            log.display()
        ),
    );
    let store = CredentialStore::with_keychain(sb.host(), Arc::new(security), sb.tunables);
    assert_eq!(
        store.read_account_credentials("1", "a@b.c", false),
        "backup-value"
    );
    let argv = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        argv.trim(),
        "find-generic-password -a account-1-a@b.c -w -s haryx"
    );
}
