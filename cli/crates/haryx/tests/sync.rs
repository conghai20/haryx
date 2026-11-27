//! `haryx sync` without a server: the shapes the apps read, and the errors before any network.
//! The full two-machine flow against a real Supabase stack is `server/supabase/tests/cli-e2e.sh`.

// The fake `security` is a `#!/bin/sh` script.
#![cfg(unix)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn haryx(home: &Path, argv: &[&str], stdin: &str) -> Output {
    let security = home.join("security");
    if !security.exists() {
        std::fs::write(&security, "#!/bin/sh\nexit 44\n").unwrap();
        std::fs::set_permissions(&security, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_haryx"))
        .args(argv)
        .current_dir(home)
        .env("HOME", home)
        .env("HARYX_HOME", home.join("store"))
        .env("HARYX_SECURITY_BIN", &security)
        .env("HARYX_SSO_ISSUER", "http://127.0.0.1:9/realms/t")
        .env("HARYX_API_URL", "http://127.0.0.1:9/api/v1")
        .env("NO_COLOR", "1")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn contract(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/contracts/haryx")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn same_keys(actual: &Value, expected: &Value) -> bool {
    let keys = |v: &Value| {
        v.as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    keys(actual) == keys(expected)
}

#[test]
fn status_signed_out_matches_the_contract_without_the_network() {
    let dir = TempDir::new().unwrap();
    let out = haryx(dir.path(), &["sync", "status", "--json"], "");
    assert!(out.status.success());
    let status = json(&out);
    assert!(
        same_keys(&status, &contract("sync-status.json")),
        "{status}"
    );
    assert_eq!(status["signedIn"], false);
    assert_eq!(status["unlocked"], false);
    assert_eq!(status["lastSyncAt"], Value::Null);
}

#[test]
fn every_network_verb_says_not_signed_in_first() {
    let dir = TempDir::new().unwrap();
    for argv in [
        &["sync", "--json"][..],
        &["sync", "has-passphrase", "--json"],
        &["sync", "unlock", "--passphrase-stdin", "--json"],
        &["sync", "forget", "a@b.c", "--json"],
        &["sync", "profile", "list", "--json"],
    ] {
        let out = haryx(dir.path(), argv, "a passphrase here\n");
        assert_eq!(out.status.code(), Some(1), "{argv:?}");
        let err = json(&out);
        assert_eq!(err["ok"], false, "{argv:?}");
        assert_eq!(err["code"], "not_signed_in", "{argv:?}");
    }
    let text = haryx(dir.path(), &["sync"], "");
    assert!(String::from_utf8_lossy(&text.stderr).contains("haryx login"));
}

#[test]
fn app_prefs_round_trip_and_reject_anything_but_an_object() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        json(&haryx(dir.path(), &["sync", "prefs", "get"], "")),
        serde_json::json!({})
    );
    let fixture = contract("app-prefs.json");
    let set = haryx(dir.path(), &["sync", "prefs", "set"], &fixture.to_string());
    assert_eq!(json(&set)["ok"], true);
    assert_eq!(
        json(&haryx(dir.path(), &["sync", "prefs", "get"], "")),
        fixture
    );
    let bad = haryx(dir.path(), &["sync", "prefs", "set"], "[1, 2]");
    assert_eq!(bad.status.code(), Some(1));
    assert_eq!(json(&bad)["code"], "invalid");
}

#[test]
fn account_delete_needs_a_session() {
    let dir = TempDir::new().unwrap();
    let out = haryx(dir.path(), &["account", "delete", "--yes", "--json"], "");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json(&out)["ok"], false);
}
