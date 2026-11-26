//! Ported from `test_account_policy_values.py` and the models cases of `test_switcher.py`.

use std::cell::RefCell;

use serde_json::json;

use super::*;

#[test]
fn valid_aliases_normalize() {
    for alias in ["dev", "work-1", "client_a", "team.b", "DEV"] {
        assert!(normalize_alias(alias).is_ok(), "{alias}");
    }
    assert_eq!(normalize_alias("  DEV ").unwrap(), "dev");
}

#[test]
fn group_names_keep_case_and_spaces_but_not_padding() {
    assert_eq!(normalize_group("  Client A ").unwrap(), "Client A");
    assert_eq!(
        normalize_group(&"x".repeat(GROUP_MAX_CHARS)).unwrap().len(),
        32
    );
    for bad in ["", "   ", "tab\there", &"x".repeat(GROUP_MAX_CHARS + 1)] {
        assert!(
            matches!(normalize_group(bad), Err(Error::Validation(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn invalid_aliases_are_rejected() {
    for alias in ["123", "dev@work", "dev work", "", "dev/work", "-dev", "   "] {
        let err = normalize_alias(alias).unwrap_err();
        assert!(matches!(err, Error::Validation(_)), "{alias}");
    }
    assert!(
        normalize_alias("123")
            .unwrap_err()
            .to_string()
            .contains("purely numeric")
    );
    assert!(
        normalize_alias("-dev")
            .unwrap_err()
            .to_string()
            .contains("start with '-'")
    );
}

#[test]
fn threshold_accepts_numbers_and_numeric_text_in_range() {
    for (raw, expected) in [(50.0, 50.0), (99.0, 99.0), (85.5, 85.5), (99.9, 99.9)] {
        assert_eq!(normalize_account_threshold(raw).unwrap(), expected);
    }
    for (raw, expected) in [
        ("85", 85.0),
        ("85.5", 85.5),
        ("  85.5  ", 85.5),
        ("50", 50.0),
        ("99.9", 99.9),
    ] {
        assert_eq!(normalize_account_threshold_str(raw).unwrap(), expected);
    }
}

#[test]
fn threshold_rejects_out_of_range() {
    for raw in [20.0, 49.9, 100.0, 99.91, 0.0, -85.0, 1000.0] {
        assert!(normalize_account_threshold(raw).is_err(), "{raw}");
    }
}

#[test]
fn threshold_rejects_non_numeric_text() {
    for raw in ["abc", "", "  ", "85%", "eighty-five", "true"] {
        assert!(normalize_account_threshold_str(raw).is_err(), "{raw}");
    }
}

#[test]
fn threshold_rejects_nan_and_infinity() {
    for raw in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(normalize_account_threshold(raw).is_err());
    }
    for raw in ["nan", "inf", "-inf"] {
        assert!(normalize_account_threshold_str(raw).is_err(), "{raw}");
    }
}

#[test]
fn threshold_error_message_names_the_valid_range() {
    let messages = [
        normalize_account_threshold(20.0).unwrap_err().to_string(),
        normalize_account_threshold(100.0).unwrap_err().to_string(),
        normalize_account_threshold(f64::NAN)
            .unwrap_err()
            .to_string(),
        normalize_account_threshold(f64::INFINITY)
            .unwrap_err()
            .to_string(),
        normalize_account_threshold_str("abc")
            .unwrap_err()
            .to_string(),
    ];
    for message in messages {
        assert!(message.contains("50"), "{message}");
        assert!(message.contains("99.9"), "{message}");
    }
    assert_eq!(
        normalize_account_threshold(20.0).unwrap_err().to_string(),
        "threshold must be between 50 and 99.9, got 20"
    );
    assert_eq!(
        normalize_account_threshold_str("abc")
            .unwrap_err()
            .to_string(),
        "threshold must be between 50 and 99.9, got 'abc'"
    );
}

#[test]
fn threshold_boundaries_are_inclusive() {
    assert_eq!(
        normalize_account_threshold(ACCOUNT_THRESHOLD_MIN).unwrap(),
        ACCOUNT_THRESHOLD_MIN
    );
    assert_eq!(
        normalize_account_threshold(ACCOUNT_THRESHOLD_MAX).unwrap(),
        ACCOUNT_THRESHOLD_MAX
    );
}

#[test]
fn threshold_constants_match_the_global_setting_spec() {
    let spec = crate::settings::setting_spec("autoswitch.threshold").unwrap();
    assert_eq!(spec.lo, Some(ACCOUNT_THRESHOLD_MIN));
    assert_eq!(spec.hi, Some(ACCOUNT_THRESHOLD_MAX));
    assert_eq!((ACCOUNT_THRESHOLD_MIN, ACCOUNT_THRESHOLD_MAX), (50.0, 99.9));
}

#[test]
fn models_remains_a_leaf() {
    let source = include_str!("mod.rs");
    let offenders: Vec<&str> = source
        .lines()
        .filter(|line| line.trim_start().starts_with("use ") && line.contains("settings"))
        .collect();
    assert!(offenders.is_empty(), "{offenders:?}");
}

#[test]
fn priority_validation() {
    assert_eq!(normalize_account_priority_str("0").unwrap(), 0);
    assert_eq!(normalize_account_priority_str(" 1000 ").unwrap(), 1000);
    assert_eq!(normalize_account_priority_str("0007").unwrap(), 7);
    assert_eq!(
        normalize_account_priority_str("00000000000000000005").unwrap(),
        5
    );
    for raw in [
        "",
        "-1",
        "+1",
        "1001",
        "1.5",
        "abc",
        "١٢",
        "99999999999999999999",
    ] {
        let err = normalize_account_priority_str(raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "priority must be an integer between 0 and 1000",
            "{raw}"
        );
    }
    assert_eq!(normalize_account_priority(3).unwrap(), 3);
    assert!(normalize_account_priority(-1).is_err());
    assert!(normalize_account_priority(1001).is_err());
}

#[test]
fn account_policy_defaults_and_equality() {
    let policy = AccountPolicy::default();
    assert_eq!(policy.threshold, None);
    assert_eq!(policy.priority, 0);
    let with = |t| AccountPolicy {
        threshold: Some(t),
        priority: 0,
    };
    assert_eq!(with(85.0), with(85.0));
    assert_ne!(with(85.0), with(90.0));
}

#[test]
fn platform_detection() {
    assert_eq!(Platform::detect_for("macos", false), Platform::MacOs);
    assert_eq!(Platform::detect_for("linux", false), Platform::Linux);
    assert_eq!(Platform::detect_for("linux", true), Platform::Wsl);
    assert_eq!(Platform::detect_for("windows", false), Platform::Windows);
    assert_eq!(Platform::detect_for("freebsd", false), Platform::Unknown);
}

fn record(value: serde_json::Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn account_info_org_fields() {
    let personal = AccountInfo::from_dict(
        1,
        &record(
            json!({"email": "user@example.com", "uuid": "user-uuid", "added": "2024-01-01T00:00:00Z"}),
        ),
    );
    assert_eq!(personal.organization_uuid, "");
    assert_eq!(personal.organization_name, "");
    assert!(!personal.is_organization());
    assert_eq!(personal.display_label(), "user@example.com [personal]");

    let org = AccountInfo::from_dict(
        2,
        &record(json!({"email": "u@e.com", "uuid": "u", "added": "",
                       "organizationUuid": "o", "organizationName": "Acme"})),
    );
    assert!(org.is_organization());
    assert_eq!(org.display_label(), "u@e.com [Acme]");
    let dict = org.to_dict();
    assert_eq!(dict["organizationUuid"], "o");
    assert_eq!(dict["organizationName"], "Acme");
    assert_eq!(
        dict.keys().collect::<Vec<_>>(),
        [
            "email",
            "uuid",
            "organizationUuid",
            "organizationName",
            "added"
        ]
    );
}

#[test]
fn null_org_fields_read_as_empty() {
    let info = AccountInfo::from_dict(
        1,
        &record(json!({"email": "a@b.c", "organizationUuid": null, "organizationName": null})),
    );
    assert_eq!(info.organization_uuid, "");
    assert_eq!(info.organization_name, "");
}

#[test]
fn org_tag_modes() {
    let emails = ["dup@x.com", "dup@x.com", "solo@x.com"];
    for (mode, email, shown) in [
        ("on", "solo@x.com", true),
        ("off", "dup@x.com", false),
        ("auto", "dup@x.com", true),
        ("auto", "solo@x.com", false),
    ] {
        assert_eq!(org_tag_shown(mode, email, &emails), shown, "{mode} {email}");
    }
}

#[test]
fn timestamp_shape() {
    let ts = get_timestamp();
    assert_eq!(ts.len(), 20, "{ts}");
    assert!(ts.ends_with('Z') && ts.as_bytes()[10] == b'T', "{ts}");
}

#[derive(Default)]
struct FakeSwitcher {
    calls: RefCell<Vec<String>>,
    fail_credentials: bool,
}

impl SwitchRollback for FakeSwitcher {
    fn write_credentials(&self, credentials: &str) -> Result<()> {
        self.calls.borrow_mut().push(format!("creds:{credentials}"));
        if self.fail_credentials {
            return Err(Error::CredentialWrite("boom".into()));
        }
        Ok(())
    }

    fn restore_active_account(&self, account_number: i64) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(format!("seq:{account_number}"));
        Ok(())
    }
}

fn transaction(config_path: PathBuf, account: &str) -> SwitchTransaction {
    SwitchTransaction {
        original_credentials: "old-creds".into(),
        original_config: "{\"old\": true}".into(),
        original_account_num: account.into(),
        original_email: "a@b.c".into(),
        config_path,
        completed_steps: Vec::new(),
    }
}

#[test]
fn rollback_undoes_steps_in_reverse_order() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".claude.json");
    std::fs::write(&config, "{\"new\": true}").unwrap();
    let mut tx = transaction(config.clone(), "2");
    for step in ["credentials_written", "config_written", "sequence_updated"] {
        tx.record_step(step);
    }
    let switcher = FakeSwitcher::default();
    assert!(tx.rollback(&switcher));
    assert_eq!(*switcher.calls.borrow(), ["seq:2", "creds:old-creds"]);
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "{\"old\": true}");
}

#[test]
fn rollback_reports_failure_but_attempts_every_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut tx = transaction(dir.path().join("c.json"), "x");
    tx.record_step("credentials_written");
    tx.record_step("sequence_updated");
    let switcher = FakeSwitcher {
        fail_credentials: true,
        ..FakeSwitcher::default()
    };
    assert!(!tx.rollback(&switcher));
    // The bad account number failed before reaching the switcher; credentials were still attempted.
    assert_eq!(*switcher.calls.borrow(), ["creds:old-creds"]);
}
