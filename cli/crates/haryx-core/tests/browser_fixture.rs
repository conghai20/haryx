//! The sync fixture the web platform's tests decrypt (`apps/platform/src/lib/fixtures/`).
//!
//! The browser rebuilds the keyring unwrap, the row-key HMAC and row decryption in TypeScript. This
//! test writes a keyring and rows with the real `SyncKey`, and on every run checks that the
//! committed fixture still opens here, so a Rust change that would strand the browser fails in Rust.
//!
//! age encryption is randomised, so the fixture is written only when it is missing or when
//! `HARYX_WRITE_BROWSER_FIXTURE=1` is set; otherwise it is only verified.

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use haryx_core::contracts::UsageSync;
use haryx_core::sync::Document;
use haryx_core::sync::crypto::SyncKey;
use haryx_core::sync::sources::label;
use serde_json::{Value, json};

const PASSPHRASE: &str = "correct horse battery staple";
const PROFILE_NAME: &str = "work";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    passphrase: String,
    /// The key the keyring wraps, `AGE-SECRET-KEY-1…`: what an unlock must recover.
    secret: String,
    /// The keyring row body as the server returns it, wrapped at the pinned `log_n` (18).
    keyring: String,
    /// The same keyring with its header claiming `log_n` 21: refused before any scrypt work.
    keyring_log_n21: String,
    check: String,
    row_keys: Vec<RowKey>,
    rows: Vec<FixtureRow>,
    profile: Profile,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RowKey {
    label: String,
    key: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRow {
    kind: String,
    key: String,
    version: i64,
    body: String,
    /// The exact plaintext the body opens to.
    plain: String,
    /// The plaintext is a `Document` naming this row (false: moved here by the server, or a body
    /// that is not a `Document`, such as a usage row).
    names_itself: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    name: String,
    object_key: String,
    /// Raw age bytes of the stored file, base64.
    file: String,
    plain: String,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/platform/src/lib/fixtures/sync-fixture.json")
}

fn usage_plain() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/contracts/haryx/usage-sync.json");
    let usage: UsageSync = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    serde_json::to_string(&usage).unwrap()
}

fn document(kind: &str, key: &str, version: i64, values: Value) -> String {
    let values: BTreeMap<String, Value> = serde_json::from_value(values).unwrap();
    serde_json::to_string(&Document {
        kind: kind.to_owned(),
        key: key.to_owned(),
        version,
        values,
        changed_at: 1_795_600_000,
    })
    .unwrap()
}

/// The same keyring with the scrypt stanza's `log_n` rewritten from 18 to `log_n`.
fn claiming(keyring: &str, log_n: u8) -> String {
    let mut ring: Value = serde_json::from_str(keyring).unwrap();
    let cipher = STANDARD.decode(ring["wrapped"].as_str().unwrap()).unwrap();
    let at = cipher.windows(10).position(|w| w == b"-> scrypt ").unwrap();
    let end = at + cipher[at..].iter().position(|&b| b == b'\n').unwrap();
    assert_eq!(&cipher[end - 3..end], b" 18");
    let mut patched = cipher[..end - 2].to_vec();
    patched.extend_from_slice(log_n.to_string().as_bytes());
    patched.extend_from_slice(&cipher[end..]);
    ring["wrapped"] = Value::String(STANDARD.encode(patched));
    ring.to_string()
}

fn generate() -> Fixture {
    let key = SyncKey::generate();
    let keyring = key.wrap(PASSPHRASE).unwrap();
    let account_label = label("account", " You@Example.com ", "org-1111");
    let account_key = key.row_key(&account_label);
    let credential_label = label("credential", "token@example.com", "");
    let credential_key = key.row_key(&credential_label);
    let usage: UsageSync = serde_json::from_str(&usage_plain()).unwrap();
    let usage_label = format!("usage:{}:{}", usage.machine_id, usage.month);
    let usage_key = key.row_key(&usage_label);
    let profile_label = format!("profile:{PROFILE_NAME}");

    let labels = [
        "keyring-check".to_owned(),
        account_label,
        credential_label,
        usage_label,
        profile_label.clone(),
        "profile:Hồ sơ 🦊".to_owned(),
        String::new(),
    ];
    let row_keys = labels
        .into_iter()
        .map(|label| RowKey {
            key: key.row_key(&label),
            label,
        })
        .collect();

    let mut rows = Vec::new();
    let mut push = |kind: &str, row_key: &str, version: i64, plain: String, names_itself| {
        rows.push(FixtureRow {
            kind: kind.to_owned(),
            key: row_key.to_owned(),
            version,
            body: key.seal(plain.as_bytes()).unwrap(),
            plain,
            names_itself,
        });
    };
    let settings = json!({"autoSwitch": true, "threshold": 90});
    push(
        "settings",
        "",
        3,
        document("settings", "", 3, settings),
        true,
    );
    let account = json!({
        "email": "you@example.com",
        "organizationUuid": "org-1111",
        "organizationName": "Nguyễn's Org",
        "alias": "Work",
        "priority": 1,
    });
    push(
        "account",
        &account_key,
        2,
        document("account", &account_key, 2, account.clone()),
        true,
    );
    let credential =
        json!({"email": "token@example.com", "token": "sk-ant-oat01-fixture-not-real"});
    push(
        "credential",
        &credential_key,
        1,
        document("credential", &credential_key, 1, credential),
        true,
    );
    push("usage", &usage_key, 1, usage_plain(), false);
    // An account document the server moved to another row: it opens, but names a different row.
    push(
        "account",
        &credential_key,
        2,
        document("account", &account_key, 2, account),
        false,
    );

    let file = "{\"profile\":\"work\",\"settings\":{}}";
    let framed = format!("haryx-profile:{PROFILE_NAME}\n{file}");
    Fixture {
        passphrase: PASSPHRASE.to_owned(),
        secret: key.to_secret(),
        keyring_log_n21: claiming(&keyring, 21),
        keyring,
        check: key.check(),
        row_keys,
        rows,
        profile: Profile {
            name: PROFILE_NAME.to_owned(),
            object_key: key.row_key(&profile_label),
            file: STANDARD.encode(key.seal_bytes(framed.as_bytes()).unwrap()),
            plain: file.to_owned(),
        },
    }
}

fn verify(fx: &Fixture) {
    let key = SyncKey::unwrap(&fx.keyring, &fx.passphrase).unwrap();
    assert_eq!(key.to_secret(), fx.secret);
    assert_eq!(key.check(), fx.check);
    assert_eq!(
        haryx_core::sync::crypto::keyring_check(&fx.keyring),
        Some(fx.check.clone())
    );
    assert!(SyncKey::unwrap(&fx.keyring, "not the passphrase at all").is_err());
    let refused = SyncKey::unwrap(&fx.keyring_log_n21, &fx.passphrase)
        .err()
        .expect("a log_n 21 keyring must be refused")
        .to_string();
    assert!(refused.contains("21") && refused.contains("haryx sync passphrase"));

    for rk in &fx.row_keys {
        assert_eq!(key.row_key(&rk.label), rk.key, "row key of {:?}", rk.label);
    }
    for row in &fx.rows {
        let plain = key.open(&row.body).expect("row opens");
        assert_eq!(String::from_utf8(plain).unwrap(), row.plain);
        let names_itself = serde_json::from_str::<Document>(&row.plain).is_ok_and(|doc| {
            doc.kind == row.kind && doc.key == row.key && doc.version == row.version
        });
        assert_eq!(names_itself, row.names_itself, "{} {}", row.kind, row.key);
    }
    let usage = fx.rows.iter().find(|r| r.kind == "usage").unwrap();
    assert_eq!(usage.plain, usage_plain());

    assert_eq!(
        key.row_key(&format!("profile:{}", fx.profile.name)),
        fx.profile.object_key
    );
    let file = key
        .open_bytes(&STANDARD.decode(&fx.profile.file).unwrap())
        .unwrap();
    let header = format!("haryx-profile:{}\n", fx.profile.name);
    assert_eq!(
        std::str::from_utf8(&file).unwrap(),
        format!("{header}{}", fx.profile.plain)
    );
}

#[test]
fn the_browser_fixture_opens_in_rust() {
    let path = fixture_path();
    if !path.exists() || std::env::var_os("HARYX_WRITE_BROWSER_FIXTURE").is_some() {
        let text = serde_json::to_string_pretty(&generate()).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text + "\n").unwrap();
    }
    let fx: Fixture = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    verify(&fx);
}
