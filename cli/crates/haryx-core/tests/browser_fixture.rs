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
use haryx_core::sync::remote::Row;
use haryx_core::sync::sources::label;
use haryx_core::sync::usage;
use serde_json::{Value, json};

const PASSPHRASE: &str = "correct horse battery staple";
const PROFILE_NAME: &str = "work";
const PROFILE_FILE: &str = "{\"profile\":\"work\",\"settings\":{}}";
/// A second machine whose usage row overlaps the contract's: the web sums them.
const OTHER_MACHINE: &str = "4c1f7a2e-0b9d-4e3a-9f6b-2d8e1a7c5b30";

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
    /// Keys of the `usage` rows `sync::usage::read_all` trusts, in its order: the web must agree.
    usage_trusted: Vec<String>,
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
    /// sealed bare instead of as a `Document`, which no reader trusts).
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

fn usage_body() -> UsageSync {
    serde_json::from_str(&usage_plain()).unwrap()
}

/// The contract's body as another machine wrote it: same day, account and model, other counts.
fn other_machine_usage() -> UsageSync {
    let mut body = usage_body();
    body.machine_id = OTHER_MACHINE.to_owned();
    body.days.truncate(1);
    let day = &mut body.days[0];
    (day.replies, day.input, day.output) = (30, 900, 100_000);
    (day.cache_read, day.cache_write5m, day.cache_write1h) = (2_000_000, 0, 50_000);
    body.quota[0].at += 1_800;
    body.quota[0].five_hour_pct = Some(57.0);
    body.quota[0].seven_day_pct = Some(12.5);
    body
}

fn usage_values(body: &UsageSync) -> Value {
    serde_json::to_value(body).unwrap()
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
    // Usage rows as `sync::usage::push` writes them: a Document whose values are the body.
    let ours = usage_body();
    push(
        "usage",
        &usage_key,
        4,
        document("usage", &usage_key, 4, usage_values(&ours)),
        true,
    );
    let theirs = other_machine_usage();
    let theirs_key = key.row_key(&usage::label(&theirs.machine_id, &theirs.month));
    push(
        "usage",
        &theirs_key,
        1,
        document("usage", &theirs_key, 1, usage_values(&theirs)),
        true,
    );
    // Names its row, but the body's machine and month derive another key: not trusted.
    let october = key.row_key(&usage::label(&ours.machine_id, "2026-10"));
    push(
        "usage",
        &october,
        1,
        document("usage", &october, 1, usage_values(&theirs)),
        true,
    );
    // A usage-share row (`usage_share::row`): the same kind, keyed per account, not a usage body.
    let share_key = key.row_key(&label("usage", "you@example.com", "org-1111"));
    let share = json!({"email": "you@example.com", "org": "org-1111", "fetchedBy": OTHER_MACHINE});
    push(
        "usage",
        &share_key,
        7,
        document("usage", &share_key, 7, share),
        true,
    );
    let prefs = json!({
        "theme": "system",
        "mascotStyles": {"you@example.com|org-1111": {"species": "cat", "hue": 0.55}},
    });
    push(
        "app-prefs",
        "",
        5,
        document("app-prefs", "", 5, prefs),
        true,
    );
    let entry =
        json!({"name": PROFILE_NAME, "bytes": PROFILE_FILE.len(), "pushedAt": 1_795_600_800});
    let index = Value::Object([(key.row_key(&profile_label), entry)].into_iter().collect());
    push("profiles", "", 2, document("profiles", "", 2, index), true);

    let file = PROFILE_FILE;
    let framed = format!("haryx-profile:{PROFILE_NAME}\n{file}");
    let usage_trusted = trusted_usage(&key, &rows);
    Fixture {
        passphrase: PASSPHRASE.to_owned(),
        secret: key.to_secret(),
        keyring_log_n21: claiming(&keyring, 21),
        keyring,
        check: key.check(),
        row_keys,
        rows,
        usage_trusted,
        profile: Profile {
            name: PROFILE_NAME.to_owned(),
            object_key: key.row_key(&profile_label),
            file: STANDARD.encode(key.seal_bytes(framed.as_bytes()).unwrap()),
            plain: file.to_owned(),
        },
    }
}

/// The keys of the usage rows `read_all` trusts, in the order it returns them.
fn trusted_usage(key: &SyncKey, rows: &[FixtureRow]) -> Vec<String> {
    let rows: Vec<Row> = rows
        .iter()
        .map(|r| Row {
            kind: r.kind.clone(),
            key: r.key.clone(),
            body: r.body.clone(),
            version: r.version,
            updated_at: 0,
        })
        .collect();
    usage::read_all(key, &rows)
        .iter()
        .map(|body| key.row_key(&usage::label(&body.machine_id, &body.month)))
        .collect()
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
    assert_eq!(trusted_usage(&key, &fx.rows), fx.usage_trusted);
    assert_eq!(
        fx.usage_trusted.len(),
        2,
        "both machines' rows, nothing else"
    );

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
