//! The active credential stays on one profile (upstream `tests/test_credentials.py`), plus the managed-key
//! axis and the global-config guard.

use super::test_support::*;
use super::*;
use crate::Error;
use serde_json::{Value, json};

const API_KEY: &str = "sk-ant-api03-kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk";

fn hashed(dir: &Path) -> String {
    keychain_service_name(dir.as_os_str())
}

// ---- reads ----------------------------------------------------------------------------------------------

#[test]
fn custom_config_dir_does_not_return_the_default_keychain_item() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));
    sb.keychain
        .put(CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE, "sk-ant-api-default");

    let result = sb.store().read_active_credentials();

    let seen = sb.keychain.services_seen(Op::Get);
    assert!(
        !seen.contains(&CLAUDE_CODE_KEYCHAIN_SERVICE.to_string()),
        "{seen:?}"
    );
    assert!(
        !seen.contains(&CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE.to_string()),
        "{seen:?}"
    );
    assert_eq!(result.value.as_deref(), Some(""));
}

#[test]
fn custom_config_dir_reads_its_own_hashed_keychain_item() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile"); // deliberately no .credentials.json
    sb.set_config_dir(&custom);
    sb.keychain.put(&hashed(&custom), &creds("custom"));
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));

    let result = sb.store().read_active_credentials();

    assert_eq!(result.value, Some(creds("custom")));
    assert_eq!(sb.keychain.services_seen(Op::Get), vec![hashed(&custom)]);
}

#[test]
fn custom_config_dir_falls_back_to_its_own_credentials_file() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    std::fs::write(custom.join(".credentials.json"), creds("custom")).unwrap();
    sb.set_config_dir(&custom);
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));

    assert_eq!(
        sb.store().read_active_credentials().value,
        Some(creds("custom"))
    );
    assert!(
        !sb.keychain
            .services_seen(Op::Get)
            .contains(&CLAUDE_CODE_KEYCHAIN_SERVICE.to_string())
    );
}

#[test]
fn default_profile_still_reads_the_unsuffixed_item() {
    let sb = Sandbox::macos();
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));
    assert_eq!(
        sb.store().read_active_credentials().value,
        Some(creds("default"))
    );
    assert_eq!(
        sb.keychain.services_seen(Op::Get),
        vec![CLAUDE_CODE_KEYCHAIN_SERVICE]
    );
}

#[test]
fn config_dir_equal_to_the_default_falls_back_to_the_unsuffixed_item() {
    let mut sb = Sandbox::macos();
    let default = sb.dir(".claude");
    sb.set_config_dir(&default);
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));

    assert_eq!(
        sb.store().read_active_credentials().value,
        Some(creds("default"))
    );
    assert_eq!(
        sb.keychain.services_seen(Op::Get),
        vec![
            hashed(&default),
            CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE.to_string(),
            CLAUDE_CODE_KEYCHAIN_SERVICE.to_string(),
        ]
    );
}

#[test]
fn secure_storage_defined_and_empty_selects_the_default_store() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.set_secure_dir(None);
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));

    assert_eq!(
        sb.store().read_active_credentials().value,
        Some(creds("default"))
    );
    assert_eq!(
        sb.keychain.services_seen(Op::Get),
        vec![CLAUDE_CODE_KEYCHAIN_SERVICE]
    );
}

#[test]
fn secure_storage_defined_and_set_selects_that_stores_hashed_item() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    let secure = sb.dir("secure-profile");
    sb.set_config_dir(&custom);
    sb.set_secure_dir(Some(&secure));
    sb.keychain.put(&hashed(&secure), &creds("secure"));
    sb.keychain.put(&hashed(&custom), &creds("custom"));
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));

    assert_eq!(
        sb.store().read_active_credentials().value,
        Some(creds("secure"))
    );
    assert_eq!(sb.keychain.services_seen(Op::Get), vec![hashed(&secure)]);
}

#[test]
fn linux_reads_the_file_then_primary_api_key_and_never_the_keychain() {
    let sb = Sandbox::linux();
    let store = sb.store();
    assert_eq!(store.read_active_credentials().value.as_deref(), Some(""));
    std::fs::write(
        sb.home().join(".claude.json"),
        json!({"primaryApiKey": API_KEY}).to_string(),
    )
    .unwrap();
    assert_eq!(store.read_credentials().as_deref(), Some(API_KEY));
    std::fs::create_dir_all(sb.home().join(".claude")).unwrap();
    std::fs::write(sb.default_cred_file(), creds("file")).unwrap();
    let active = store.read_active_credentials();
    assert_eq!(active.value, Some(creds("file")));
    assert!(!active.degraded && !active.keychain_unavailable);
    assert!(sb.keychain.all_calls().is_empty());
}

#[cfg(unix)]
#[test]
fn an_unreadable_credentials_file_is_a_read_error() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::linux();
    std::fs::create_dir_all(sb.home().join(".claude")).unwrap();
    let file = sb.default_cred_file();
    std::fs::write(&file, creds("x")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&file).is_ok() {
        return; // running as root
    }
    assert_eq!(sb.store().read_active_credentials().value, None);
}

// ---- writes ---------------------------------------------------------------------------------------------

#[test]
fn a_write_round_trips_through_the_read_under_a_custom_profile() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.keychain.put(&hashed(&custom), &creds("custom"));

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();
    assert_eq!(store.read_credentials(), Some(creds("target")));
}

#[test]
fn a_write_under_a_custom_profile_leaves_the_default_login_alone() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));
    sb.keychain.put(&hashed(&custom), &creds("custom"));

    sb.store().write_credentials(&creds("target")).unwrap();

    assert_eq!(
        sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT),
        Some(creds("default"))
    );
}

#[test]
fn a_custom_profile_gets_a_seed_file_with_no_item_shadowing_it() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.keychain.put(&hashed(&custom), &creds("custom"));

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();

    assert_eq!(
        std::fs::read_to_string(custom.join(".credentials.json")).unwrap(),
        creds("target")
    );
    assert_eq!(sb.keychain.get(&hashed(&custom), ACCOUNT), None);
    assert_eq!(
        store.last_active_credentials_backend(),
        Some(CredentialBackend::File)
    );
}

#[test]
fn config_dir_equal_to_the_default_does_not_write_the_unsuffixed_item() {
    let mut sb = Sandbox::macos();
    let default = sb.dir(".claude");
    sb.set_config_dir(&default);
    sb.keychain.put(&hashed(&default), &creds("default"));

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();

    assert_eq!(store.read_credentials(), Some(creds("target")));
    assert_eq!(sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT), None);
}

#[test]
fn the_default_profile_still_writes_the_unsuffixed_item() {
    let sb = Sandbox::macos();
    sb.dir(".claude");
    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();

    assert_eq!(
        sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT),
        Some(creds("target"))
    );
    assert_eq!(
        store.last_active_credentials_backend(),
        Some(CredentialBackend::Keychain)
    );
    assert!(!sb.default_cred_file().exists());
}

#[test]
fn a_keychain_write_refreshes_an_existing_file_but_never_creates_one() {
    let sb = Sandbox::macos();
    std::fs::create_dir_all(sb.home().join(".claude")).unwrap();
    std::fs::write(sb.default_cred_file(), creds("old")).unwrap();
    sb.store().write_credentials(&creds("new")).unwrap();
    assert_eq!(
        std::fs::read_to_string(sb.default_cred_file()).unwrap(),
        creds("new")
    );
}

#[test]
fn the_same_store_follows_claudes_rotation_after_the_write() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.keychain.put(&hashed(&custom), &creds("custom"));

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();
    assert_eq!(store.read_credentials(), Some(creds("target")));

    sb.keychain.put(&hashed(&custom), &creds("rotated"));
    assert_eq!(store.read_credentials(), Some(creds("rotated")));
}

#[test]
fn a_clean_custom_profile_write_keeps_backups_in_the_keychain() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    sb.keychain.put(&hashed(&custom), &creds("custom"));

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();

    assert!(store.use_keychain());
    assert!(!store.uses_file_backup_backend());
    assert_eq!(store.residual_verdict(), None);
}

#[test]
fn a_failed_delete_under_a_custom_profile_still_pins_file_mode() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    sb.set_config_dir(&custom);
    let item = hashed(&custom);
    sb.keychain.put(&item, &creds("custom"));
    sb.keychain.fail_when(|op, _, _| op == Op::Delete);

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();

    assert!(sb.keychain.get(&item, ACCOUNT).is_some());
    assert!(!store.use_keychain());
    assert_eq!(store.residual_verdict(), Some(false));
    let active = store.read_active_credentials();
    assert_eq!(active.value, Some(creds("target")));
    assert!(active.degraded);
}

#[test]
fn a_secure_storage_override_gets_the_seed_claude_reads() {
    let mut sb = Sandbox::macos();
    let custom = sb.dir("custom-profile");
    let secure = sb.dir("secure-profile");
    sb.set_config_dir(&custom);
    sb.set_secure_dir(Some(&secure));
    sb.keychain.put(&hashed(&secure), &creds("secure"));
    sb.keychain.put(&hashed(&custom), &creds("custom"));
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));

    let store = sb.store();
    store.write_credentials(&creds("target")).unwrap();

    assert_eq!(
        std::fs::read_to_string(secure.join(".credentials.json")).unwrap(),
        creds("target")
    );
    assert!(!custom.join(".credentials.json").exists());
    assert_eq!(sb.keychain.get(&hashed(&secure), ACCOUNT), None);
    assert_eq!(
        sb.keychain.get(&hashed(&custom), ACCOUNT),
        Some(creds("custom"))
    );
    assert_eq!(
        sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT),
        Some(creds("default"))
    );
    assert_eq!(store.read_credentials(), Some(creds("target")));
}

#[cfg(unix)]
#[test]
fn the_live_file_is_written_0600() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::linux();
    sb.store().write_credentials(&creds("x")).unwrap();
    let mode = std::fs::metadata(sb.default_cred_file())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

// ---- explicit default profile: two logins in play -------------------------------------------------------

fn two_logins(sb: &mut Sandbox) -> (PathBuf, String) {
    let default = sb.dir(".claude");
    sb.set_config_dir(&default);
    let item = hashed(&default);
    sb.keychain.put(&item, &creds("custom"));
    sb.keychain
        .put(CLAUDE_CODE_KEYCHAIN_SERVICE, &creds("default"));
    (default, item)
}

#[test]
fn an_oauth_switch_leaves_the_other_login_in_place() {
    let mut sb = Sandbox::macos();
    let (default, item) = two_logins(&mut sb);
    let store = sb.store();
    assert_eq!(store.read_credentials(), Some(creds("custom")));
    store.write_credentials(&creds("target")).unwrap();

    assert_eq!(
        sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT),
        Some(creds("default"))
    );
    assert_eq!(sb.keychain.get(&item, ACCOUNT), None);
    assert_eq!(
        std::fs::read_to_string(default.join(".credentials.json")).unwrap(),
        creds("target")
    );
    assert_eq!(store.read_credentials(), Some(creds("target")));
}

#[test]
fn a_new_process_reads_the_switched_login_not_the_other_one() {
    let mut sb = Sandbox::macos();
    two_logins(&mut sb);
    sb.store().write_credentials(&creds("target")).unwrap();
    assert_eq!(sb.store().read_credentials(), Some(creds("target")));
}

#[test]
fn an_api_key_switch_leaves_the_other_login_in_place() {
    let mut sb = Sandbox::macos();
    let (default, item) = two_logins(&mut sb);
    let store = sb.store();
    store.write_credentials(API_KEY).unwrap();

    assert_eq!(
        sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT),
        Some(creds("default"))
    );
    assert_eq!(sb.keychain.get(&item, ACCOUNT), None);
    assert!(!default.join(".credentials.json").exists());
    assert_eq!(store.read_credentials().as_deref(), Some(API_KEY));
    assert_eq!(sb.store().read_credentials().as_deref(), Some(API_KEY));
}

#[test]
fn a_login_only_in_the_unsuffixed_item_is_still_read_and_kept() {
    let mut sb = Sandbox::macos();
    let (_default, item) = two_logins(&mut sb);
    sb.keychain.remove(&item);
    let store = sb.store();
    assert_eq!(store.read_credentials(), Some(creds("default")));
    store.write_credentials(&creds("target")).unwrap();

    assert_eq!(
        sb.keychain.get(CLAUDE_CODE_KEYCHAIN_SERVICE, ACCOUNT),
        Some(creds("default"))
    );
    assert_eq!(store.read_credentials(), Some(creds("target")));
    assert_eq!(sb.store().read_credentials(), Some(creds("target")));
}

#[test]
fn a_failed_key_lookup_does_not_report_an_empty_login() {
    let mut sb = Sandbox::macos();
    let (_default, item) = two_logins(&mut sb);
    sb.keychain.remove(&item);
    sb.keychain.fail_when(|op, service, _| {
        op == Op::Get && service == CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE
    });

    let active = sb.store().read_active_credentials();

    assert_eq!(active.value.as_deref(), Some(""));
    assert!(active.keychain_unavailable);
    assert!(active.degraded);
}

// ---- managed key axis -----------------------------------------------------------------------------------

fn global_config(sb: &Sandbox) -> Value {
    serde_json::from_str(&std::fs::read_to_string(sb.home().join(".claude.json")).unwrap()).unwrap()
}

#[test]
fn an_api_key_on_linux_goes_to_primary_api_key_and_clears_oauth() {
    let sb = Sandbox::linux();
    std::fs::write(
        sb.home().join(".claude.json"),
        json!({"oauthAccount": {"emailAddress": "a@b.c"}, "projects": {"p": 1}}).to_string(),
    )
    .unwrap();
    let store = sb.store();
    store.write_credentials(&creds("oauth")).unwrap();
    assert!(sb.default_cred_file().exists());

    store.write_credentials(&format!("  {API_KEY}\n")).unwrap();

    let cfg = global_config(&sb);
    assert_eq!(cfg["primaryApiKey"], API_KEY);
    assert_eq!(
        cfg["customApiKeyResponses"]["approved"],
        json!([approved_form(API_KEY)])
    );
    assert_eq!(cfg["customApiKeyResponses"]["rejected"], json!([]));
    assert_eq!(cfg["oauthAccount"]["emailAddress"], "a@b.c");
    assert_eq!(cfg["projects"]["p"], 1);
    assert!(!sb.default_cred_file().exists());
    assert_eq!(
        store.last_active_credentials_backend(),
        Some(CredentialBackend::File)
    );

    // Back to OAuth: primaryApiKey dropped, `approved` kept.
    store.write_credentials(&creds("oauth2")).unwrap();
    let cfg = global_config(&sb);
    assert!(cfg.get("primaryApiKey").is_none());
    assert_eq!(
        cfg["customApiKeyResponses"]["approved"],
        json!([approved_form(API_KEY)])
    );
    assert_eq!(store.read_credentials(), Some(creds("oauth2")));
}

#[test]
fn an_api_key_on_macos_goes_to_the_keychain_not_the_config() {
    let sb = Sandbox::macos();
    let store = sb.store();
    store.write_credentials(API_KEY).unwrap();
    assert_eq!(
        sb.keychain
            .get(CLAUDE_CODE_MANAGED_KEYCHAIN_SERVICE, ACCOUNT)
            .as_deref(),
        Some(API_KEY)
    );
    let cfg = global_config(&sb);
    assert!(cfg.get("primaryApiKey").is_none());
    assert_eq!(
        store.last_active_credentials_backend(),
        Some(CredentialBackend::Keychain)
    );
    assert_eq!(store.read_credentials().as_deref(), Some(API_KEY));
}

#[test]
fn a_failed_managed_key_keychain_write_falls_back_and_pins_unverified() {
    let sb = Sandbox::macos();
    sb.keychain.fail_when(|op, _, _| op == Op::Set);
    let store = sb.store();
    store.write_credentials(API_KEY).unwrap();
    assert_eq!(global_config(&sb)["primaryApiKey"], API_KEY);
    assert!(!store.use_keychain());
    assert_eq!(store.residual_verdict(), Some(false));
}

#[test]
fn approving_the_same_key_twice_records_it_once() {
    let sb = Sandbox::linux();
    let store = sb.store();
    store.write_credentials(API_KEY).unwrap();
    store.write_credentials(API_KEY).unwrap();
    assert_eq!(
        global_config(&sb)["customApiKeyResponses"]["approved"],
        json!([approved_form(API_KEY)])
    );
}

#[test]
fn an_unreadable_global_config_is_never_overwritten() {
    let sb = Sandbox::linux();
    let torn = r#"{"oauthAccount": {"emailAddress": "a@b.c"}, "projects": {"#;
    std::fs::write(sb.home().join(".claude.json"), torn).unwrap();
    let store = sb.store();

    let err = store.write_credentials(API_KEY).unwrap_err();
    assert!(matches!(err, Error::CredentialWrite(_)), "{err:?}");
    store.clear_managed_key(); // warns, does not rewrite
    assert_eq!(
        std::fs::read_to_string(sb.home().join(".claude.json")).unwrap(),
        torn
    );
}

#[test]
fn an_absent_global_config_is_created_by_an_update() {
    let sb = Sandbox::linux();
    sb.store()
        .update_global_config(|cfg| {
            cfg.insert("k".into(), json!(1));
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(sb.home().join(".claude.json")).unwrap(),
        "{\n  \"k\": 1\n}"
    );
}
