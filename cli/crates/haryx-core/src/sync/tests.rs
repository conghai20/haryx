//! Two machines, one in-memory server: the engine end to end.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use super::profiles::MemoryObjects;
use super::remote::{Memory, Remote, Row};
use super::*;
use crate::settings::set_setting;
use crate::switcher::test_support::{TestSwitcher, sample};

const PASS: &str = "correct horse battery";
const USER: &str = "user-1";
const PROJECT: &str = "https://p.supabase.co";

struct Machine {
    sw: TestSwitcher,
    vault: Vault,
    _dir: tempfile::TempDir,
}

impl Machine {
    fn new(roster: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        Machine {
            sw: TestSwitcher::linux().with_roster(roster),
            vault: Vault::file(dir.path().join("account")),
            _dir: dir,
        }
    }

    fn ctx<'a>(&'a self, remote: &'a dyn Remote, objects: &'a MemoryObjects) -> Context<'a> {
        Context {
            root: self.sw.backup_dir(),
            switcher: &self.sw,
            vault: &self.vault,
            remote,
            objects,
            user: USER,
            project: PROJECT,
            now: 1_800_000_000,
        }
    }

    fn account(&self, num: &str) -> Value {
        self.sw.roster()["accounts"][num].clone()
    }
}

struct World {
    server: Memory,
    objects: MemoryObjects,
}

impl World {
    fn new() -> Self {
        World {
            server: Memory::default(),
            objects: MemoryObjects::default(),
        }
    }

    fn sync(&self, m: &Machine) -> Report {
        run(&m.ctx(&self.server, &self.objects)).unwrap()
    }

    fn unlock(&self, m: &Machine) -> Unlocked {
        unlock(&m.ctx(&self.server, &self.objects), PASS).unwrap()
    }
}

#[test]
fn the_first_machine_sets_the_passphrase_and_the_next_must_know_it() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    assert!(w.unlock(&a).created);
    let wrong = unlock(&b.ctx(&w.server, &w.objects), "not the passphrase");
    assert_eq!(wrong.unwrap_err(), SyncError::WrongPassphrase);
    assert!(!w.unlock(&b).created);
    assert!(is_unlocked(&b.vault, USER, PROJECT));
    assert!(!is_unlocked(&b.vault, "someone-else", PROJECT));
}

#[test]
fn a_locked_machine_cannot_sync() {
    let w = World::new();
    let a = Machine::new(sample());
    let err = run(&a.ctx(&w.server, &w.objects)).unwrap_err();
    assert_eq!(err, SyncError::Locked);
    assert_eq!(err.code(), "locked");
}

#[test]
fn settings_travel_both_ways_and_local_only_ones_stay() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    set_setting(a.sw.backup_dir(), "autoswitch.threshold", "90").unwrap();
    set_setting(a.sw.backup_dir(), "history.import", "none").unwrap();
    assert_eq!(w.sync(&a).pushed, 4, "two accounts, settings and app prefs");
    let report = w.sync(&b);
    assert!(
        report.changed.contains(&"settings".to_owned()),
        "{report:?}"
    );
    let (b_settings, _) = sources::read_settings(b.sw.backup_dir()).unwrap();
    assert_eq!(b_settings.get("autoswitch.threshold"), Some(&json!(90.0)));
    assert!(!b_settings.contains_key("history.import"), "local only");

    crate::settings::unset_setting(b.sw.backup_dir(), "autoswitch.threshold").unwrap();
    w.sync(&b);
    w.sync(&a);
    let (a_settings, _) = sources::read_settings(a.sw.backup_dir()).unwrap();
    assert!(
        !a_settings.contains_key("autoswitch.threshold"),
        "an unset travels too"
    );
}

#[test]
fn account_settings_follow_and_accounts_elsewhere_are_listed() {
    let w = World::new();
    let only_on_a = json!({
        "activeAccountNumber": 1, "sequence": [1, 2, 3],
        "accounts": {
            "1": {"email": "account1@example.com", "uuid": "uuid-1", "alias": "work", "priority": 2},
            "2": {"email": "account2@example.com", "uuid": "uuid-2"},
            "3": {"email": "only-a@example.com", "uuid": "uuid-3", "alias": "spare",
                  "organizationName": "Acme"},
        },
    });
    let (a, b) = (Machine::new(only_on_a), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    w.sync(&a);
    let report = w.sync(&b);
    assert_eq!(b.account("1")["alias"], "work");
    assert_eq!(b.account("1")["priority"], 2);
    assert_eq!(
        report.other_machine_accounts,
        [state::RemoteAccount {
            email: "only-a@example.com".into(),
            organization_name: "Acme".into(),
            alias: "spare".into()
        }]
    );
    let status = status(b.sw.backup_dir(), &b.vault, Some((USER, PROJECT)));
    assert_eq!(status.other_machine_accounts.len(), 1);
    assert!(status.unlocked && status.last_sync_at.is_some());

    let forgotten = forget(&b.ctx(&w.server, &w.objects), "Only-A@example.com").unwrap();
    assert_eq!(forgotten, 1);
    assert!(
        status_of(&b).other_machine_accounts.is_empty(),
        "forgotten here at once"
    );
}

fn status_of(m: &Machine) -> Status {
    status(m.sw.backup_dir(), &m.vault, Some((USER, PROJECT)))
}

#[test]
fn the_server_sees_neither_emails_nor_values() {
    let w = World::new();
    let a = Machine::new(sample());
    w.unlock(&a);
    set_setting(a.sw.backup_dir(), "autoswitch.strategy", "best").unwrap();
    w.sync(&a);
    for row in w.server.snapshot() {
        for secret in ["account1@example.com", "best", "autoswitch"] {
            assert!(!row.body.contains(secret), "{} leaks {secret}", row.kind);
            assert!(!row.key.contains(secret));
        }
        assert!(row.key.is_empty() || row.key.len() == 64);
    }
}

#[test]
fn app_preferences_are_passed_along_untouched() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    let prefs: merge::Values = [
        ("theme".to_owned(), json!("dark")),
        ("macos.menuBarPercent".to_owned(), json!(true)),
        (
            "mascotStyles".to_owned(),
            json!({"account1@example.com": {"species": "otter"}}),
        ),
    ]
    .into_iter()
    .collect();
    sources::write_app_prefs(a.sw.backup_dir(), &prefs).unwrap();
    w.sync(&a);
    let report = w.sync(&b);
    assert!(report.changed.contains(&"app-prefs".to_owned()));
    assert_eq!(sources::read_app_prefs(b.sw.backup_dir()).unwrap().0, prefs);
}

#[test]
fn token_accounts_arrive_working_and_a_removal_here_is_respected() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    let key = "sk-ant-api03-test-key-value";
    let stored =
        a.sw.store_token_account(key, Some("ci@example.com"), None, true)
            .unwrap()
            .unwrap();
    assert!(!stored.updated);
    w.sync(&a);
    let report = w.sync(&b);
    assert!(
        report.changed.contains(&"credential".to_owned()),
        "{report:?}"
    );
    let roster = b.sw.roster();
    let (num, _) = roster["accounts"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, r)| r["email"] == "ci@example.com")
        .expect("the token account arrived");
    assert_eq!(
        b.sw.read_account_credentials(num, "ci@example.com", false),
        key
    );
    let logged = serde_json::to_string(&report.overwritten).unwrap();
    assert!(!logged.contains(key));
    for m in [&a, &b] {
        let state = std::fs::read_to_string(State::path(m.sw.backup_dir())).unwrap();
        assert!(
            !state.contains(key),
            "state.json keeps a digest, not the token"
        );
    }
    assert!(
        w.sync(&a).pushed == 0 && w.sync(&b).pushed == 0,
        "and stays settled"
    );

    // Removed on B on purpose: the next sync must not bring it back.
    b.sw.remove_account(num, true).unwrap();
    w.sync(&b);
    let back = b.sw.roster()["accounts"]
        .as_object()
        .unwrap()
        .values()
        .any(|r| r["email"] == "ci@example.com");
    assert!(!back, "a removed token account stays removed");
}

#[test]
fn browser_logins_never_reach_the_server() {
    let w = World::new();
    let a = Machine::new(sample());
    a.sw.write_account_credentials(
        "1",
        "account1@example.com",
        &crate::switcher::test_support::creds("at", 1),
    )
    .unwrap();
    w.unlock(&a);
    w.sync(&a);
    assert!(
        w.server
            .snapshot()
            .iter()
            .all(|r| r.kind != KIND_CREDENTIAL),
        "an OAuth login is not a credential row"
    );
}

#[test]
fn a_reset_elsewhere_locks_this_machine_until_it_unlocks_again() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    w.sync(&a);
    w.sync(&b);
    reset(&a.ctx(&w.server, &w.objects)).unwrap();
    assert!(!is_unlocked(&a.vault, USER, PROJECT));
    assert_eq!(
        run(&b.ctx(&w.server, &w.objects)).unwrap_err(),
        SyncError::KeyChanged
    );
    assert!(
        !is_unlocked(&b.vault, USER, PROJECT),
        "B forgot the old key"
    );

    // A sets a new passphrase and uploads again; B joins with it and nothing is lost.
    assert!(w.unlock(&a).created);
    w.sync(&a);
    assert!(!w.unlock(&b).created);
    w.sync(&b);
    assert_eq!(b.account("1")["email"], "account1@example.com");
}

#[test]
fn changing_the_passphrase_keeps_the_key() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    change_passphrase(&a.ctx(&w.server, &w.objects), "a brand new passphrase").unwrap();
    assert_eq!(
        unlock(&b.ctx(&w.server, &w.objects), PASS).unwrap_err(),
        SyncError::WrongPassphrase
    );
    unlock(&b.ctx(&w.server, &w.objects), "a brand new passphrase").unwrap();
    w.sync(&a);
    w.sync(&b);
}

/// A server where another machine writes first, once.
struct Contended<'a> {
    inner: &'a Memory,
    raced: AtomicBool,
}

impl Remote for Contended<'_> {
    fn list(&self) -> Result<Vec<Row>, SyncError> {
        self.inner.list()
    }
    fn insert(&self, kind: &str, key: &str, body: &str) -> Result<Option<Row>, SyncError> {
        self.inner.insert(kind, key, body)
    }
    fn update(&self, kind: &str, key: &str, body: &str, v: i64) -> Result<Option<Row>, SyncError> {
        if !self.raced.swap(true, Ordering::SeqCst) {
            let current = self.inner.list()?;
            let row = current
                .iter()
                .find(|r| r.kind == kind && r.key == key)
                .unwrap();
            self.inner.update(kind, key, &row.body, row.version)?;
        }
        self.inner.update(kind, key, body, v)
    }
    fn delete(&self, kind: &str, key: &str) -> Result<(), SyncError> {
        self.inner.delete(kind, key)
    }
    fn delete_all(&self) -> Result<(), SyncError> {
        self.inner.delete_all()
    }
}

#[test]
fn a_write_that_loses_a_race_starts_over_and_succeeds() {
    let w = World::new();
    let a = Machine::new(sample());
    w.unlock(&a);
    w.sync(&a);
    set_setting(a.sw.backup_dir(), "autoswitch.threshold", "80").unwrap();
    let contended = Contended {
        inner: &w.server,
        raced: AtomicBool::new(false),
    };
    run(&a.ctx(&contended, &w.objects)).unwrap();
    assert!(contended.raced.load(Ordering::SeqCst));
}

#[test]
fn profiles_round_trip_encrypted_and_are_capped() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    let ctx_a = a.ctx(&w.server, &w.objects);
    profiles::push(&ctx_a, "work", b"profile bytes").unwrap();
    let stored = w.objects.files.lock().unwrap().clone();
    let (name, bytes) = stored.iter().next().unwrap();
    assert!(name.starts_with("user-1/") && !name.contains("work"));
    assert!(!String::from_utf8_lossy(bytes).contains("profile bytes"));

    let ctx_b = b.ctx(&w.server, &w.objects);
    assert_eq!(profiles::list(&ctx_b).unwrap()[0].name, "work");
    assert_eq!(
        profiles::pull(&ctx_b, "work").unwrap().as_deref(),
        Some(&b"profile bytes"[..])
    );
    assert_eq!(profiles::pull(&ctx_b, "nope").unwrap(), None);
    profiles::push(&ctx_a, "two", b"2").unwrap();
    profiles::push(&ctx_a, "three", b"3").unwrap();
    profiles::push(&ctx_a, "work", b"replaced").unwrap();
    assert!(matches!(
        profiles::push(&ctx_a, "four", b"4"),
        Err(SyncError::Invalid(_))
    ));
    assert!(profiles::remove(&ctx_b, "two").unwrap());
    assert!(!profiles::remove(&ctx_b, "two").unwrap());
    assert_eq!(profiles::list(&ctx_a).unwrap().len(), 2);

    // The server hands back one profile's file under another's name: refused, not imported.
    {
        let mut files = w.objects.files.lock().unwrap();
        let names: Vec<String> = files.keys().cloned().collect();
        let first = files[&names[0]].clone();
        files.insert(names[1].clone(), first);
    }
    let pulled: Vec<_> = ["work", "three"]
        .iter()
        .map(|n| profiles::pull(&ctx_b, n))
        .collect();
    assert!(pulled.iter().any(Result::is_err), "{pulled:?}");

    reset(&ctx_a).unwrap();
    assert!(
        w.objects.files.lock().unwrap().is_empty(),
        "reset removes the files too"
    );
}

#[test]
fn a_value_one_machine_cannot_take_is_not_removed_everywhere() {
    let w = World::new();
    let on_a = json!({
        "activeAccountNumber": 1, "sequence": [1, 2],
        "accounts": {
            "1": {"email": "account1@example.com", "uuid": "uuid-1", "alias": "home"},
            "2": {"email": "account2@example.com", "uuid": "uuid-2"},
        },
    });
    let on_b = json!({
        "activeAccountNumber": 1, "sequence": [1, 2],
        "accounts": {
            "1": {"email": "account1@example.com", "uuid": "uuid-1"},
            "2": {"email": "account2@example.com", "uuid": "uuid-2", "alias": "home"},
        },
    });
    let (a, b) = (Machine::new(on_a), Machine::new(on_b));
    w.unlock(&a);
    w.unlock(&b);
    w.sync(&a);
    let report = w.sync(&b);
    assert!(
        report.skipped.iter().any(|s| s.contains("alias")),
        "{report:?}"
    );
    assert!(
        b.account("1").get("alias").is_none(),
        "taken here by account 2"
    );
    w.sync(&b);
    w.sync(&a);
    assert_eq!(a.account("1")["alias"], "home", "A keeps it");
}

#[test]
fn a_synced_token_never_replaces_a_browser_login() {
    let w = World::new();
    let only_2 = json!({
        "activeAccountNumber": 2, "sequence": [2],
        "accounts": {"2": {"email": "account2@example.com", "uuid": "uuid-2"}},
    });
    let (a, b) = (Machine::new(only_2), Machine::new(sample()));
    let login = crate::switcher::test_support::creds("browser-at", 1);
    b.sw.write_account_credentials("1", "account1@example.com", &login)
        .unwrap();
    let before =
        b.sw.read_account_credentials("1", "account1@example.com", false);
    a.sw.store_token_account(
        "sk-ant-api03-for-account-1",
        Some("account1@example.com"),
        None,
        true,
    )
    .unwrap();
    w.unlock(&a);
    w.unlock(&b);
    w.sync(&a);
    let report = w.sync(&b);
    assert!(
        report.skipped.iter().any(|s| s.contains("own sign-in")),
        "{report:?}"
    );
    assert_eq!(
        b.sw.read_account_credentials("1", "account1@example.com", false),
        before
    );
}

#[test]
fn a_row_that_does_not_open_is_replaced_not_a_lockout() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    w.sync(&a);
    let settings = w
        .server
        .snapshot()
        .into_iter()
        .find(|r| r.kind == KIND_SETTINGS)
        .unwrap();
    w.server
        .update(KIND_SETTINGS, "", "garbage", settings.version)
        .unwrap()
        .unwrap();
    w.sync(&b);
    assert!(is_unlocked(&b.vault, USER, PROJECT), "B keeps its key");
    let row = w
        .server
        .snapshot()
        .into_iter()
        .find(|r| r.kind == KIND_SETTINGS)
        .unwrap();
    assert_ne!(row.body, "garbage", "replaced with B's copy");
    w.sync(&a);
}

#[test]
fn a_replayed_old_row_is_not_taken() {
    let w = World::new();
    let (a, b) = (Machine::new(sample()), Machine::new(sample()));
    w.unlock(&a);
    w.unlock(&b);
    set_setting(a.sw.backup_dir(), "autoswitch.threshold", "80").unwrap();
    w.sync(&a);
    let old = w
        .server
        .snapshot()
        .into_iter()
        .find(|r| r.kind == KIND_SETTINGS)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    set_setting(a.sw.backup_dir(), "autoswitch.threshold", "90").unwrap();
    w.sync(&a);
    w.sync(&b);
    let now = w
        .server
        .snapshot()
        .into_iter()
        .find(|r| r.kind == KIND_SETTINGS)
        .unwrap();
    // The server puts the old body back under a raised version.
    w.server
        .update(KIND_SETTINGS, "", &old.body, now.version)
        .unwrap()
        .unwrap();
    w.sync(&b);
    let (b_settings, _) = sources::read_settings(b.sw.backup_dir()).unwrap();
    assert_eq!(b_settings.get("autoswitch.threshold"), Some(&json!(90.0)));
}
