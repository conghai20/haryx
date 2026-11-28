//! `haryx account consents|consent|export|machines|machine rename`, the consents in `whoami`, the
//! machine heartbeat in `sync` and the remote sign-out message, end to end: the real binary signed
//! in against a fake Keycloak and a fake HaryX API (`common`).

// The fakes are `#!/bin/sh` scripts.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering;

use common::*;
use serde_json::{Value, json};

const THIS_MACHINE: &str = "9b2e5c1a-4f0d-4a67-8d3e-1c7a5b0e2f91";

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// This machine's id, set before the CLI makes one.
fn with_machine_id(s: &Sandbox) {
    let cache = s.home().join("store/cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("machine-id"), THIS_MACHINE).unwrap();
}

#[test]
fn whoami_shows_the_consents_as_the_contract_has_them() {
    let s = Sandbox::signed_in();
    s.api().set(|st| st.sync = true);
    let status = &json_lines(&s.haryx(&["whoami", "--json"], ""))[0];
    assert!(same_shape(status, &contract("whoami.json")), "{status}");
    assert_eq!(
        status["consents"],
        json!([
            {"purpose": "sync", "granted": true, "version": "2026-11-01"},
            {"purpose": "machines", "granted": false, "version": null},
        ])
    );
    let text = stdout(&s.haryx(&["whoami"], ""));
    assert!(
        text.contains("Sync: on. Shown on HaryX web: off."),
        "{text}"
    );
}

#[test]
fn consents_are_listed_granted_and_withdrawn_with_confirmation() {
    let s = Sandbox::signed_in();
    let list = &json_lines(&s.haryx(&["account", "consents", "--json"], ""))[0];
    assert!(same_shape(list, &contract("consents.json")), "{list}");
    assert_eq!(list["consents"][1]["granted"], false);

    let out = s.haryx(
        &[
            "account", "consent", "grant", "machines", "--source", "macos", "--json",
        ],
        "",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json_lines(&out)[0]["consents"][1]["granted"], true);
    let sent: Value = serde_json::from_str(&s.api().sent("PUT", "/consents/machines")[0]).unwrap();
    assert_eq!(sent, json!({"version": "2026-11-01", "source": "macos"}));
    assert_eq!(
        s.api().sent("PUT", "/machines/self").len(),
        1,
        "the machine shows at once"
    );

    let text = stdout(&s.haryx(&["account", "consent", "grant", "sync"], ""));
    assert!(text.contains("Sync is on for your HaryX ID (privacy policy 2026-11-01)."));
    assert_eq!(
        serde_json::from_str::<Value>(&s.api().sent("PUT", "/consents/sync")[0]).unwrap()["source"],
        "cli"
    );

    // Not a terminal, no --yes: nothing is withdrawn, and the answer says what would be erased.
    let out = s.haryx(&["account", "consent", "withdraw", "sync", "--json"], "");
    assert_eq!(out.status.code(), Some(1));
    let refused = &json_lines(&out)[0];
    assert_eq!(refused["ok"], false);
    let message = refused["message"].as_str().unwrap();
    assert!(message.contains("deletes HaryX's server copy") && message.contains("--yes"));
    assert!(s.api().requests().iter().all(|(m, _, _)| m != "DELETE"));

    let out = s.haryx(
        &["account", "consent", "withdraw", "sync", "--yes", "--json"],
        "",
    );
    assert!(out.status.success());
    assert_eq!(json_lines(&out)[0]["consents"][0]["granted"], false);
    assert_eq!(s.api().sent("DELETE", "/consents/sync?source=cli").len(), 1);
}

#[test]
fn export_writes_the_apis_file_readable_only_by_the_user() {
    let s = Sandbox::signed_in();
    let out = s.haryx(&["account", "export", "mine.json", "--json"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = s.home().join("mine.json");
    let answer = &json_lines(&out)[0];
    assert_eq!(answer["ok"], true);
    assert!(answer["path"].as_str().unwrap().ends_with("/mine.json"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"exportedAt":"2026-11-27T00:00:00Z"}"#
    );
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let text = stdout(&s.haryx(&["account", "export"], ""));
    assert!(text.contains("Saved your HaryX data to"), "{text}");
    let default = std::fs::read_dir(s.home())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .find(|name| name.starts_with("haryx-export-") && name.ends_with(".json"));
    assert!(default.is_some(), "./haryx-export-DATE.json");
}

#[test]
fn machines_mark_this_one_by_its_id_and_rename() {
    let s = Sandbox::signed_in();
    with_machine_id(&s);
    s.api()
        .set(|st| st.machine_ids = vec![THIS_MACHINE.into(), OTHER_MACHINE.into()]);
    let list = &json_lines(&s.haryx(&["account", "machines", "--json"], ""))[0];
    assert!(same_shape(list, &contract("machines.json")), "{list}");
    assert_eq!(list["machines"][0]["machineId"], THIS_MACHINE);
    assert_eq!(list["machines"][0]["isThisMachine"], true);
    assert_eq!(list["machines"][1]["isThisMachine"], false);
    let text = stdout(&s.haryx(&["account", "machines"], ""));
    assert!(text.contains("Box (this machine)"), "{text}");

    let out = s.haryx(
        &[
            "account",
            "machine",
            "rename",
            OTHER_MACHINE,
            "Home PC",
            "--json",
        ],
        "",
    );
    assert!(out.status.success());
    assert_eq!(json_lines(&out)[0]["name"], "Home PC");
    assert_eq!(
        s.api().sent("PATCH", &format!("/machines/{OTHER_MACHINE}")),
        [r#"{"name":"Home PC"}"#]
    );

    let unknown = "00000000-0000-4000-8000-000000000000";
    let out = s.haryx(
        &["account", "machine", "rename", unknown, "X", "--json"],
        "",
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json_lines(&out)[0]["code"], "invalid");
}

#[test]
fn sync_heartbeats_once_while_the_machines_consent_is_on() {
    let s = Sandbox::signed_in();
    with_machine_id(&s);
    s.api().set(|st| st.machines = true);
    // Sync itself is locked here; the heartbeat goes first, whatever the pass does.
    let out = s.haryx(&["sync", "--json"], "");
    assert_eq!(json_lines(&out)[0]["code"], "locked");
    let beats = s.api().sent("PUT", "/machines/self");
    assert_eq!(beats.len(), 1);
    let beat: Value = serde_json::from_str(&beats[0]).unwrap();
    assert_eq!(beat["machineId"], THIS_MACHINE);
    assert_eq!(beat["version"], env!("CARGO_PKG_VERSION"));
    assert!(["macos", "linux"].contains(&beat["os"].as_str().unwrap()));
    assert!(!beat["name"].as_str().unwrap().is_empty());

    // Off: the API refuses the heartbeat, and the sync goes on as if it had not been sent.
    s.api().set(|st| st.machines = false);
    let out = s.haryx(&["sync", "--json"], "");
    let lines = json_lines(&out);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["code"], "locked");
}

#[test]
fn sync_without_the_sync_consent_says_what_to_run() {
    let s = Sandbox::signed_in();
    let out = s.haryx(&["sync", "has-passphrase", "--json"], "");
    assert_eq!(out.status.code(), Some(1));
    let answer = &json_lines(&out)[0];
    assert_eq!(answer["code"], "consent_required");
    let message = answer["message"].as_str().unwrap();
    assert!(
        message.contains("Run 'haryx account consent grant sync'"),
        "{message}"
    );
}

#[test]
fn a_sign_out_from_the_web_says_so_and_how_to_sign_in_again() {
    let s = Sandbox::signed_in();
    s.api().set(|st| st.revoked = true);
    s.keycloak.token_status.store(400, Ordering::SeqCst);
    let out = s.haryx(&["sync", "--json"], "");
    let answer = &json_lines(&out)[0];
    assert_eq!(answer["code"], "signed_out");
    let message = answer["message"].as_str().unwrap();
    assert!(
        message.starts_with("Signed out of HaryX: ")
            && message.contains("on the web")
            && message.contains("WeCoAp account page")
            && message.contains("Run 'haryx login'"),
        "{message}"
    );
    assert!(!s.stored_session().exists());

    let s = Sandbox::signed_in();
    s.api().set(|st| st.revoked = true);
    s.keycloak.token_status.store(400, Ordering::SeqCst);
    let text = stdout(&s.haryx(&["whoami"], ""));
    assert!(
        text.contains("Not signed in to HaryX: this machine's sign-in has ended"),
        "{text}"
    );
    let status = &json_lines(&s.haryx(&["whoami", "--json"], ""))[0];
    assert_eq!(status["signedIn"], false);
}
