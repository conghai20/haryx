//! `swap` and `move`, ported from upstream `tests/test_swap_accounts.py` and `test_move_accounts.py`.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};

use super::test_support::*;
use crate::credentials::keychain_service_name;
use crate::credentials::test_support::ACCOUNT;
use crate::error::Error;

const A1: &str = "account1@example.com";
const A2: &str = "account2@example.com";
const ORG_EMAIL: &str = "user@example.com";

fn sw() -> TestSwitcher {
    TestSwitcher::linux().with_roster(sample())
}

fn org_sw() -> TestSwitcher {
    TestSwitcher::linux().with_roster(sample_with_org())
}

fn read(s: &TestSwitcher, num: &str, email: &str) -> String {
    s.read_account_credentials(num, email, false)
}

fn staging_files(s: &TestSwitcher) -> Vec<String> {
    std::fs::read_dir(&s.credentials_dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(".swap-staging-"))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(unix)]
fn not_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() != 0 }
}

// --- swap ------------------------------------------------------------------------------------------

#[test]
fn swap_by_number() {
    let s = sw();
    assert_eq!(s.swap_accounts("1", "2").unwrap(), ("1".into(), "2".into()));
    assert_eq!(s.email_at("1"), A2);
    assert_eq!(s.email_at("2"), A1);
}

#[test]
fn swap_moves_active_number_with_account_and_keeps_sequence_sorted() {
    let s = sw();
    s.swap_accounts("1", "2").unwrap();
    let data = s.roster();
    assert_eq!(data["activeAccountNumber"], json!(2));
    assert_eq!(data["sequence"], json!([1, 2]));
}

#[test]
fn swap_by_email_and_alias_moves_the_alias() {
    let mut roster = sample();
    roster["accounts"]["2"]["alias"] = json!("dev");
    let s = TestSwitcher::linux().with_roster(roster);
    assert_eq!(
        s.swap_accounts(A1, "dev").unwrap(),
        ("1".into(), "2".into())
    );
    let data = s.roster();
    assert_eq!(data["accounts"]["1"]["alias"], json!("dev"));
    assert!(data["accounts"]["2"].get("alias").is_none());
}

#[test]
fn swap_moves_credential_and_config_backups() {
    let s = sw();
    s.write_account_credentials("1", A1, "creds-one").unwrap();
    s.write_account_config("1", A1, "config-one").unwrap();
    s.write_account_credentials("2", A2, "creds-two").unwrap();
    s.write_account_config("2", A2, "config-two").unwrap();
    s.swap_accounts("1", "2").unwrap();
    assert_eq!(read(&s, "2", A1), "creds-one");
    assert_eq!(s.read_account_config("2", A1).unwrap(), "config-one");
    assert_eq!(read(&s, "1", A2), "creds-two");
    assert_eq!(s.read_account_config("1", A2).unwrap(), "config-two");
    assert_eq!(read(&s, "1", A1), "");
    assert_eq!(read(&s, "2", A2), "");
}

#[test]
fn swap_with_one_slot_missing_backups() {
    let s = sw();
    s.write_account_credentials("1", A1, "creds-one").unwrap();
    s.swap_accounts("1", "2").unwrap();
    assert_eq!(read(&s, "2", A1), "creds-one");
    assert_eq!(read(&s, "1", A2), "");
}

#[test]
fn swap_rejects_self_and_unknown() {
    let s = sw();
    assert!(matches!(
        s.swap_accounts("1", "1"),
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        s.swap_accounts("1", "nosuch@example.com"),
        Err(Error::AccountNotFound(_))
    ));
}

#[test]
fn swap_without_roster_is_a_config_error() {
    let s = TestSwitcher::linux();
    let err = s.swap_accounts("1", "2").unwrap_err();
    assert!(matches!(err, Error::Config(ref m) if m == "No accounts are managed yet"));
}

#[test]
fn swap_same_email_accounts() {
    let s = org_sw();
    s.write_account_credentials("1", ORG_EMAIL, "creds-org")
        .unwrap();
    s.write_account_credentials("2", ORG_EMAIL, "creds-personal")
        .unwrap();
    s.swap_accounts("1", "2").unwrap();
    assert_eq!(read(&s, "1", ORG_EMAIL), "creds-personal");
    assert_eq!(read(&s, "2", ORG_EMAIL), "creds-org");
    assert_eq!(
        s.roster()["accounts"]["2"]["organizationUuid"],
        json!("org-uuid-5678")
    );
    assert!(staging_files(&s).is_empty());
    // The displaced generations are not left behind as `.prev` copies.
    let prevs = std::fs::read_dir(&s.credentials_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".enc.prev"))
        .count();
    assert_eq!(prevs, 0);
}

#[test]
fn swap_same_email_partial_failure_rolls_back() {
    let s = org_sw();
    s.write_account_credentials("1", ORG_EMAIL, "creds-org")
        .unwrap();
    s.write_account_credentials("2", ORG_EMAIL, "creds-personal")
        .unwrap();
    s.faults.fail_credential_writes_from(2, true);
    assert!(matches!(s.swap_accounts("1", "2"), Err(Error::Io(_))));
    s.faults.heal();
    assert_eq!(read(&s, "1", ORG_EMAIL), "creds-org");
    assert_eq!(read(&s, "2", ORG_EMAIL), "creds-personal");
    let data = s.roster();
    assert_eq!(
        data["accounts"]["1"]["organizationUuid"],
        json!("org-uuid-5678")
    );
    assert_eq!(data["activeAccountNumber"], json!(1));
}

#[test]
#[cfg(unix)]
fn swap_same_email_persistent_failure_keeps_staged_copy() {
    let s = org_sw();
    s.write_account_credentials("1", ORG_EMAIL, "creds-org")
        .unwrap();
    s.write_account_credentials("2", ORG_EMAIL, "creds-personal")
        .unwrap();
    s.faults.fail_credential_writes_from(2, false);
    assert!(s.swap_accounts("1", "2").is_err());
    s.faults.heal();
    assert_eq!(read(&s, "1", ORG_EMAIL), "creds-org");
    let staged = s.credentials_dir.join(".swap-staging-creds-2.json");
    assert_eq!(std::fs::read_to_string(&staged).unwrap(), "creds-personal");
    let mode = std::fs::metadata(&staged).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn swap_same_email_rollback_restores_empty_slot() {
    let s = org_sw();
    s.write_account_credentials("1", ORG_EMAIL, "creds-org")
        .unwrap();
    s.faults.fail_write_json(true);
    assert!(s.swap_accounts("1", "2").is_err());
    s.faults.heal();
    assert_eq!(read(&s, "1", ORG_EMAIL), "creds-org");
    assert_eq!(read(&s, "2", ORG_EMAIL), "");
    assert_eq!(
        s.roster()["accounts"]["1"]["organizationUuid"],
        json!("org-uuid-5678")
    );
    assert!(staging_files(&s).is_empty());
}

#[test]
fn swap_same_email_one_sided_clears_destination() {
    let s = org_sw();
    s.write_account_credentials("1", ORG_EMAIL, "creds-org")
        .unwrap();
    s.swap_accounts("1", "2").unwrap();
    assert_eq!(read(&s, "2", ORG_EMAIL), "creds-org");
    assert_eq!(read(&s, "1", ORG_EMAIL), "");
}

#[test]
fn swap_clears_stale_destination_key() {
    let s = sw();
    s.write_account_credentials("2", A1, "stale-foreign")
        .unwrap();
    s.swap_accounts("1", "2").unwrap();
    assert_eq!(read(&s, "2", A1), "");
}

#[test]
fn swap_refuses_leftover_staging() {
    let s = org_sw();
    s.write_account_credentials("1", ORG_EMAIL, "creds-org")
        .unwrap();
    s.write_account_credentials("2", ORG_EMAIL, "creds-personal")
        .unwrap();
    let leftover = s.credentials_dir.join(".swap-staging-creds-1.json");
    std::fs::write(&leftover, "only-surviving-copy").unwrap();
    let err = s.swap_accounts("1", "2").unwrap_err();
    assert!(err.to_string().contains("interrupted swap"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&leftover).unwrap(),
        "only-surviving-copy"
    );
    assert_eq!(read(&s, "1", ORG_EMAIL), "creds-org");
    assert_eq!(read(&s, "2", ORG_EMAIL), "creds-personal");
}

#[test]
fn swap_moves_session_profiles() {
    let s = sw();
    let session_a = s.session_dir("1", A1);
    std::fs::create_dir_all(&session_a).unwrap();
    std::fs::write(session_a.join("marker.txt"), "history-of-account-one").unwrap();
    s.swap_accounts("1", "2").unwrap();
    let moved = s.session_dir("2", A1);
    assert_eq!(
        std::fs::read_to_string(moved.join("marker.txt")).unwrap(),
        "history-of-account-one"
    );
    assert!(!session_a.exists());
}

#[test]
#[cfg(unix)]
fn swap_aborts_on_unreadable_backup_before_anything_changes() {
    if !not_root() {
        return;
    }
    let s = sw();
    s.write_account_credentials("1", A1, "rt-1").unwrap();
    s.write_account_credentials("2", A2, "rt-2").unwrap();
    let enc = s.backup_enc_path("2", A2);
    std::fs::set_permissions(&enc, std::fs::Permissions::from_mode(0o000)).unwrap();
    let err = s.swap_accounts("1", "2").unwrap_err();
    std::fs::set_permissions(&enc, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(err.to_string().contains("could not be read"), "{err}");
    assert_eq!(s.email_at("1"), A1);
    assert_eq!(read(&s, "1", A1), "rt-1");
    assert_eq!(read(&s, "2", A2), "rt-2");
}

#[test]
fn swap_deletes_both_old_paths_session_keychain_entries() {
    let s = TestSwitcher::macos().with_roster(sample());
    let kc = s.keychain();
    let mut services = Vec::new();
    for (num, email) in [("1", A1), ("2", A2)] {
        let dir = s.session_dir(num, email);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".credentials.json"), "plaintext-seed").unwrap();
        let service = keychain_service_name(dir.as_os_str());
        kc.put_for(&service, ACCOUNT, &format!("rotated-{num}"));
        services.push(service);
    }
    s.swap_accounts("1", "2").unwrap();
    assert!(s.session_dir("2", A1).exists());
    assert!(s.session_dir("1", A2).exists());
    for service in services {
        assert_eq!(kc.get(&service, ACCOUNT), None);
    }
}

fn seed_profile(
    s: &TestSwitcher,
    num: &str,
    email: &str,
    backup: &str,
    profile: &str,
    identity: Option<&str>,
) {
    s.store
        .write_account_credentials(num, email, backup)
        .unwrap();
    let dir = s.session_dir(num, email);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".credentials.json"), profile).unwrap();
    let default_identity =
        json!({"oauthAccount": {"emailAddress": email, "organizationUuid": ""}}).to_string();
    std::fs::write(
        dir.join(".claude.json"),
        identity.unwrap_or(&default_identity),
    )
    .unwrap();
}

#[test]
fn swap_carries_both_profiles_newer_generations() {
    let s = sw();
    for (num, email) in [("1", A1), ("2", A2)] {
        seed_profile(
            &s,
            num,
            email,
            &creds(&format!("stored-{num}"), 1),
            &creds(&format!("fresh-{num}"), 9_999_999_999_999),
            None,
        );
    }
    s.swap_accounts("1", "2").unwrap();
    assert_eq!(read(&s, "2", A1), creds("fresh-1", 9_999_999_999_999));
    assert_eq!(read(&s, "1", A2), creds("fresh-2", 9_999_999_999_999));
}

#[test]
fn swap_refuses_when_an_ahead_profile_cannot_be_adopted() {
    let s = sw();
    seed_profile(
        &s,
        "1",
        A1,
        &creds("stored-1", 1),
        &creds("fresh-1", 9_999_999_999_999),
        Some("not json{{{"),
    );
    seed_profile(
        &s,
        "2",
        A2,
        &creds("stored-2", 1),
        &creds("fresh-2", 9_999_999_999_999),
        None,
    );
    let err = s.swap_accounts("1", "2").unwrap_err();
    assert!(matches!(err, Error::Session(ref m) if m.contains("could not be adopted")));
    assert_eq!(s.email_at("1"), A1);
    assert_eq!(read(&s, "1", A1), creds("stored-1", 1));
    assert!(s.session_dir("1", A1).exists());
}

// --- move ------------------------------------------------------------------------------------------

#[test]
fn move_to_empty_slot_relocates_and_renumbers() {
    let s = sw();
    assert_eq!(
        s.move_account("2", "5").unwrap(),
        ("2".into(), "5".into(), false)
    );
    let data = s.roster();
    assert_eq!(data["accounts"]["5"]["email"], json!(A2));
    assert!(data["accounts"].get("2").is_none());
    assert_eq!(data["sequence"], json!([1, 5]));
}

#[test]
fn move_keeps_sequence_sorted_and_follows_active() {
    let s = sw();
    s.move_account("1", "9").unwrap();
    let data = s.roster();
    assert_eq!(data["sequence"], json!([2, 9]));
    assert_eq!(data["activeAccountNumber"], json!(9));
}

#[test]
fn move_by_alias_moves_the_alias() {
    let mut roster = sample();
    roster["accounts"]["2"]["alias"] = json!("dev");
    let s = TestSwitcher::linux().with_roster(roster);
    assert_eq!(
        s.move_account("dev", "7").unwrap(),
        ("2".into(), "7".into(), false)
    );
    assert_eq!(s.roster()["accounts"]["7"]["alias"], json!("dev"));
}

#[test]
fn move_relocates_backups_and_session_profile() {
    let s = sw();
    s.write_account_credentials("2", A2, "creds-two").unwrap();
    s.write_account_config("2", A2, "config-two").unwrap();
    let session = s.session_dir("2", A2);
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(session.join("marker.txt"), "history").unwrap();
    s.move_account("2", "5").unwrap();
    assert_eq!(read(&s, "5", A2), "creds-two");
    assert_eq!(s.read_account_config("5", A2).unwrap(), "config-two");
    assert_eq!(read(&s, "2", A2), "");
    assert_eq!(s.read_account_config("2", A2).unwrap(), "");
    assert!(s.session_dir("5", A2).join("marker.txt").exists());
    assert!(!session.exists());
}

#[test]
fn move_unbacked_account_clears_stale_target_key() {
    let s = sw();
    s.write_account_credentials("5", A2, "stale-foreign")
        .unwrap();
    s.move_account("2", "5").unwrap();
    assert_eq!(read(&s, "5", A2), "");
}

#[test]
#[cfg(unix)]
fn move_failed_required_clear_aborts_commit() {
    if !not_root() {
        return;
    }
    let s = sw();
    s.write_account_credentials("5", A2, "stale-foreign")
        .unwrap();
    let dir = s.credentials_dir.clone();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = s.move_account("2", "5");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let err = result.unwrap_err();
    assert!(err.to_string().contains("aborting before commit"), "{err}");
    let data = s.roster();
    assert_eq!(data["accounts"]["2"]["email"], json!(A2));
    assert!(data["accounts"].get("5").is_none());
}

#[test]
fn move_metadata_failure_leaves_account_intact() {
    let s = sw();
    s.write_account_credentials("2", A2, "creds-two").unwrap();
    s.faults.fail_write_json(true);
    assert!(s.move_account("2", "5").is_err());
    s.faults.heal();
    assert_eq!(read(&s, "2", A2), "creds-two");
    assert_eq!(read(&s, "5", A2), "");
    let data = s.roster();
    assert_eq!(data["accounts"]["2"]["email"], json!(A2));
    assert!(data["accounts"].get("5").is_none());
}

#[test]
fn move_to_occupied_slot_swaps_like_swap() {
    let s = sw();
    assert_eq!(
        s.move_account("1", "2").unwrap(),
        ("1".into(), "2".into(), true)
    );
    assert_eq!(s.email_at("2"), A1);
    assert_eq!(s.email_at("1"), A2);
}

#[test]
fn move_to_same_slot_is_noop_and_padded_target_normalizes() {
    let s = sw();
    assert_eq!(
        s.move_account("1", "1").unwrap(),
        ("1".into(), "1".into(), false)
    );
    let (_, target, _) = s.move_account("2", "05").unwrap();
    assert_eq!(target, "5");
    assert_eq!(s.email_at("5"), A2);
}

#[test]
fn move_rejects_invalid_targets_and_unknown_accounts() {
    let s = sw();
    for bad in ["abc", "0", "-1", "1.5", ""] {
        assert!(
            matches!(s.move_account("1", bad), Err(Error::Validation(_))),
            "{bad}"
        );
    }
    assert!(matches!(
        s.move_account("nosuch@example.com", "5"),
        Err(Error::AccountNotFound(_))
    ));
    let err = s.move_account("1", "100").unwrap_err();
    assert!(err.to_string().contains("out of range"));
}

#[test]
fn move_cap_stretches_to_existing_max_slot() {
    let mut roster = sample();
    roster["accounts"]["150"] = json!({"email": "account150@example.com", "uuid": "uuid-150"});
    roster["sequence"]
        .as_array_mut()
        .unwrap()
        .push(Value::from(150));
    let s = TestSwitcher::linux().with_roster(roster);
    assert_eq!(
        s.move_account("1", "120").unwrap(),
        ("1".into(), "120".into(), false)
    );
    let err = s.move_account("2", "151").unwrap_err();
    assert!(err.to_string().contains("out of range"));
}

#[test]
fn move_carries_the_profiles_newer_generation() {
    let s = sw();
    seed_profile(
        &s,
        "2",
        A2,
        &creds("stored", 1),
        &creds("fresh", 9_999_999_999_999),
        None,
    );
    s.move_account("2", "5").unwrap();
    assert_eq!(read(&s, "5", A2), creds("fresh", 9_999_999_999_999));
}

#[test]
fn move_keeps_a_backup_that_is_ahead_of_the_profile() {
    let s = sw();
    seed_profile(
        &s,
        "2",
        A2,
        &creds("relogin", 9_999_999_999_999),
        &creds("old", 1),
        None,
    );
    s.move_account("2", "5").unwrap();
    assert_eq!(read(&s, "5", A2), creds("relogin", 9_999_999_999_999));
}

#[test]
fn relocation_refuses_inside_a_session_shell() {
    let mut s = sw();
    let inside = s.backup_dir.join("sessions").join("1-test");
    std::fs::create_dir_all(&inside).unwrap();
    s.sandbox.set_config_dir(&inside);
    let s = s.rebuilt();
    assert!(matches!(s.swap_accounts("1", "2"), Err(Error::Switch(_))));
    assert!(matches!(s.move_account("2", "5"), Err(Error::Switch(_))));
}
