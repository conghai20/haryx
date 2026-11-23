//! The roster, JSON I/O and identifier resolution, ported from upstream `tests/test_switcher.py`
//! (`TestResolveAccountIdentifier`, `TestResolveByAlias`, `TestResolveIdentifierAmbiguity`,
//! `TestUpgradeMigration`, `TestSalvageCopyMode`) and the `_read_json` / `_write_json` cases.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use super::Switcher;
use super::test_support::*;
use crate::error::Error;
use crate::oauth::AccountIdentity;

fn sw() -> TestSwitcher {
    TestSwitcher::linux().with_roster(sample())
}

#[test]
#[cfg(unix)]
fn init_sequence_file_creates_an_empty_roster_once() {
    let s = TestSwitcher::linux();
    s.setup_directories().unwrap();
    assert_eq!(s.get_sequence_data().unwrap(), None);
    s.init_sequence_file().unwrap();
    let data = s.roster();
    assert_eq!(data["sequence"], json!([]));
    assert_eq!(data["activeAccountNumber"], json!(null));
    s.write_roster(sample());
    s.init_sequence_file().unwrap();
    assert_eq!(s.roster()["sequence"], json!([1, 2]));
    let mode = std::fs::metadata(&s.sequence_file)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn torn_roster_is_a_config_error_not_empty() {
    let s = TestSwitcher::linux();
    s.setup_directories().unwrap();
    std::fs::write(&s.sequence_file, "{\"accounts\": ").unwrap();
    assert!(matches!(s.get_sequence_data(), Err(Error::Config(_))));
    std::fs::write(&s.sequence_file, "123").unwrap();
    let err = s.get_sequence_data().unwrap_err();
    assert!(
        err.to_string().contains("holds int, not a JSON object"),
        "{err}"
    );
    // The lenient reader answers None for both.
    assert_eq!(s.read_json(&s.sequence_file, false).unwrap(), None);
}

#[test]
fn next_account_number_is_one_past_the_highest() {
    let s = TestSwitcher::linux();
    assert_eq!(s.get_next_account_number().unwrap(), 1);
    let mut roster = sample();
    roster["accounts"]["7"] = json!({"email": "x@example.com"});
    let s = s.with_roster(roster);
    assert_eq!(s.get_next_account_number().unwrap(), 8);
}

#[test]
fn write_json_is_python_layout_and_ascii() {
    let s = TestSwitcher::linux();
    s.setup_directories().unwrap();
    let path = s.backup_dir.join("x.json");
    let map = json!({"a": 1, "b": "é"}).as_object().unwrap().clone();
    s.write_json(&path, &map).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"a\": 1,\n  \"b\": \"\\u00e9\"\n}"
    );
}

#[test]
#[cfg(unix)]
fn salvage_copy_is_private_and_never_overwrites() {
    let s = TestSwitcher::linux();
    let path = s.claude_config_path();
    std::fs::write(&path, r#"{"primaryApiKey": "sk-test", "torn":"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut warnings = Vec::new();
    let first = s.salvage_unreadable(&path, false, &mut warnings).unwrap();
    let second = s.salvage_unreadable(&path, false, &mut warnings).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&path).unwrap()
    );
    let mode = std::fs::metadata(&first).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].starts_with(".claude.json could not be parsed — a copy was kept at"));
}

#[test]
fn resolve_by_number_email_and_nonexistent() {
    let s = sw();
    assert_eq!(s.resolve_account_identifier("1").unwrap(), Some("1".into()));
    assert_eq!(
        s.resolve_account_identifier("account2@example.com")
            .unwrap(),
        Some("2".into())
    );
    assert_eq!(
        s.resolve_account_identifier("nobody@example.com").unwrap(),
        None
    );
    // A digit string is returned as given, existing or not.
    assert_eq!(s.resolve_account_identifier("9").unwrap(), Some("9".into()));
}

#[test]
fn resolve_by_alias_case_insensitive_with_precedence() {
    let mut roster = sample();
    roster["accounts"]["2"]["alias"] = json!("work");
    // An alias equal to another account's email wins over that email.
    roster["accounts"]["1"]["alias"] = json!("account2@example.com");
    let s = TestSwitcher::linux().with_roster(roster);
    assert_eq!(
        s.resolve_account_identifier("WORK").unwrap(),
        Some("2".into())
    );
    assert_eq!(
        s.resolve_account_identifier("account2@example.com")
            .unwrap(),
        Some("1".into())
    );
    assert_eq!(s.resolve_account_identifier("2").unwrap(), Some("2".into()));
}

#[test]
fn empty_identifier_never_matches_an_aliasless_account() {
    let s = sw();
    assert_eq!(s.find_account_by_alias("").unwrap(), None);
    assert_eq!(s.resolve_account_identifier("").unwrap(), None);
}

#[test]
fn ambiguous_email_is_a_config_error() {
    let s = TestSwitcher::linux().with_roster(sample_with_org());
    let err = s
        .resolve_account_identifier("user@example.com")
        .unwrap_err();
    assert!(matches!(err, Error::Config(_)));
    assert_eq!(
        err.to_string(),
        "Email 'user@example.com' is ambiguous — matches accounts: 1 [Acme Corp], 2 [personal]. Use \
         account number instead (e.g., haryx --switch-to 1)."
    );
    assert_eq!(s.resolve_account_identifier("1").unwrap(), Some("1".into()));
}

#[test]
fn resolve_account_returns_identity_and_errors() {
    let s = TestSwitcher::linux().with_roster(sample_with_org());
    assert_eq!(
        s.resolve_account("1").unwrap(),
        (
            "1".into(),
            "user@example.com".into(),
            "org-uuid-5678".into()
        )
    );
    assert!(
        matches!(s.resolve_account("5"), Err(Error::AccountNotFound(m)) if m == "Account-5 does not exist")
    );
    assert!(matches!(
        s.resolve_account("x@y.com"),
        Err(Error::AccountNotFound(m)) if m == "No account found with identifier: x@y.com"
    ));
}

#[test]
fn org_fields_are_backfilled_from_live_config_then_backups() {
    let s = TestSwitcher::linux().with_roster(json!({
        "activeAccountNumber": 1, "sequence": [1, 2, 3],
        "accounts": {
            "1": {"email": "user@example.com"},
            "2": {"email": "other@example.com"},
            "3": {"email": "third@example.com"},
        },
    }));
    s.set_live("user@example.com", "org-live");
    s.write_account_config(
        "2",
        "other@example.com",
        &json!({"oauthAccount": {"organizationUuid": "org-2", "organizationName": "Two"}})
            .to_string(),
    )
    .unwrap();
    let data = s.get_sequence_data_migrated().unwrap().unwrap();
    assert_eq!(data["accounts"]["1"]["organizationUuid"], json!("org-live"));
    assert_eq!(data["accounts"]["2"]["organizationUuid"], json!("org-2"));
    assert_eq!(data["accounts"]["2"]["organizationName"], json!("Two"));
    assert_eq!(data["accounts"]["3"]["organizationUuid"], json!(""));
    // Idempotent: a second pass writes nothing new.
    let before = std::fs::read_to_string(&s.sequence_file).unwrap();
    s.get_sequence_data_migrated().unwrap();
    assert_eq!(std::fs::read_to_string(&s.sequence_file).unwrap(), before);
}

#[test]
fn find_account_slot_and_exists_use_the_composite_key() {
    let s = TestSwitcher::linux().with_roster(sample_with_org());
    let data = s.roster();
    assert_eq!(
        Switcher::find_account_slot(&data, "user@example.com", ""),
        Some("2".into())
    );
    assert_eq!(
        Switcher::find_account_slot(&data, "user@example.com", "org-uuid-5678"),
        Some("1".into())
    );
    assert!(!s.account_exists("user@example.com", "other-org").unwrap());
}

#[test]
fn current_account_number_needs_a_managed_live_login() {
    let s = sw();
    assert!(!s.has_live_login());
    assert_eq!(s.current_account_number().unwrap(), None);
    s.set_live("account2@example.com", "");
    assert!(s.has_live_login());
    assert_eq!(s.current_account_number().unwrap(), Some("2".into()));
    assert!(s.live_identity_matches("account2@example.com", ""));
    s.set_live("stranger@example.com", "");
    assert_eq!(s.current_account_number().unwrap(), None);
}

#[test]
fn account_kind_defaults_to_oauth() {
    let mut roster = sample();
    roster["accounts"]["2"]["kind"] = json!("api_key");
    let s = TestSwitcher::linux().with_roster(roster);
    assert_eq!(s.account_kind(None).unwrap(), "oauth");
    assert_eq!(s.account_kind_for("1").unwrap(), "oauth");
    assert_eq!(s.account_kind_for("2").unwrap(), "api_key");
    let err = s
        .reject_cross_kind_collision("account2@example.com", false)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("already exists as an API-key account (slot 2)")
    );
    s.reject_cross_kind_collision("account2@example.com", true)
        .unwrap();
}

#[test]
fn resolved_identity_matching_is_uuid_first() {
    let mut roster = sample();
    roster["accounts"]["2"]["uuid"] = json!("");
    roster["accounts"]["2"]["organizationUuid"] = json!("");
    let s = TestSwitcher::linux().with_roster(roster);
    let id = |uuid: &str, email: Option<&str>, org: Option<&str>| AccountIdentity {
        uuid: uuid.into(),
        email: email.map(Into::into),
        organization_uuid: org.map(Into::into),
    };
    assert_eq!(
        s.resolved_matches_slot_identity("1", &id("uuid-1", None, None))
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        s.resolved_matches_slot_identity("1", &id("uuid-x", None, None))
            .unwrap(),
        Some(false)
    );
    // Uuid-less slot: an exact (email, org) match affirms and backfills the uuid.
    assert_eq!(
        s.resolved_matches_slot_identity("2", &id("new-uuid", Some("account2@example.com"), None))
            .unwrap(),
        None
    );
    assert_eq!(
        s.resolved_matches_slot_identity(
            "2",
            &id("new-uuid", Some("account2@example.com"), Some(""))
        )
        .unwrap(),
        Some(true)
    );
    assert_eq!(s.account_identity("2").unwrap().uuid, "new-uuid");
}

#[test]
fn backfill_never_rewrites_an_existing_uuid() {
    let s = sw();
    s.backfill_account_uuid("1", "other", None, None).unwrap();
    assert_eq!(s.account_identity("1").unwrap().uuid, "uuid-1");
}

#[test]
fn foreign_capture_guard() {
    let s = sw();
    let live = creds("tok", 9_999_999_999_999);
    let resolves = |uuid: &str, email: &str, org: Option<&str>| {
        let identity = AccountIdentity {
            uuid: uuid.into(),
            email: Some(email.into()),
            organization_uuid: org.map(Into::into),
        };
        move |_: &str| Some(identity)
    };
    // Wiped: refused without asking the oracle.
    let wiped = json!({"claudeAiOauth": {"accessToken": "", "refreshToken": ""}}).to_string();
    let err = s
        .check_foreign_capture(&wiped, "a@x.com", "", "", |_| unreachable!())
        .unwrap_err();
    assert!(err.to_string().contains("has no access or refresh token"));
    // Expired: unverifiable, registered anyway.
    let expired = creds("tok", 1);
    assert_eq!(
        s.check_foreign_capture(&expired, "a@x.com", "", "", |_| unreachable!())
            .unwrap(),
        expired
    );
    // Uuid disagreement refuses; agreement with a matching org passes.
    let err = s
        .check_foreign_capture(
            &live,
            "a@x.com",
            "",
            "u-1",
            resolves("u-2", "a@x.com", Some("")),
        )
        .unwrap_err();
    assert!(err.to_string().contains("resolves to account u-2, not u-1"));
    s.check_foreign_capture(
        &live,
        "a@x.com",
        "",
        "u-1",
        resolves("u-1", "a@x.com", Some("")),
    )
    .unwrap();
    // Without a stored uuid the email decides (case-insensitively).
    let err = s
        .check_foreign_capture(&live, "a@x.com", "", "", resolves("u", "b@x.com", None))
        .unwrap_err();
    assert!(err.to_string().contains("the token resolves to b@x.com"));
    s.check_foreign_capture(&live, "A@x.com", "", "", resolves("u", "a@x.com", None))
        .unwrap();
    // Same address, different org.
    let err = s
        .check_foreign_capture(
            &live,
            "a@x.com",
            "org-1",
            "",
            resolves("u", "a@x.com", Some("")),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("belongs to organization personal, not org-1")
    );
}

#[test]
fn live_api_key_capture_is_refused() {
    let s = sw();
    let err = s
        .reject_live_api_key_capture("sk-ant-api03-xyz")
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
    s.reject_live_api_key_capture(&creds("t", 1)).unwrap();
}

#[test]
fn tag_suffix_follows_the_org_tag_mode() {
    let emails = ["a@x.com", "a@x.com", "b@x.com"];
    assert_eq!(
        Switcher::tag_suffix("off", &emails, "a@x.com", "Acme", "o"),
        ""
    );
    assert!(Switcher::tag_suffix("auto", &emails, "a@x.com", "Acme", "o").contains("[Acme]"));
    assert_eq!(Switcher::tag_suffix("auto", &emails, "b@x.com", "", ""), "");
    assert!(Switcher::tag_suffix("on", &emails, "b@x.com", "", "").contains("[personal]"));
}

#[test]
fn validate_email_shapes() {
    assert!(Switcher::validate_email("a.b+c@example.co"));
    assert!(!Switcher::validate_email("a@b"));
    assert!(!Switcher::validate_email("@example.com"));
    assert!(!Switcher::validate_email("a@example.c"));
}
