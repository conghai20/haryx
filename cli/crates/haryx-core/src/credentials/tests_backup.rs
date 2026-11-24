//! Slot backups, `.prev` retention, the display read cache and cooldown (upstream
//! `test_macos_keychain_contract.py::TestBackupCredentialsSecurity`, `test_backup_read_pacing.py`), and the
//! unclaimed stash.

use super::test_support::*;
use super::*;
use base64::Engine as _;
use serde_json::{Map, Value, json};

const EMAIL2: &str = "user2@example.com";
const EMAIL3: &str = "user3@example.com";

fn backup_account(num: &str, email: &str) -> String {
    format!("account-{num}-{email}")
}

/// A macOS sandbox with Keychain backups for slots 2 and 3, and a registry file.
fn seeded() -> Sandbox {
    let sb = Sandbox::macos();
    for (num, email) in [("2", EMAIL2), ("3", EMAIL3)] {
        sb.keychain.put_for(
            HARYX_BACKUP_SERVICE,
            &backup_account(num, email),
            &creds(&format!("backup{num}")),
        );
    }
    std::fs::create_dir_all(sb.store_root()).unwrap();
    std::fs::write(
        sb.store_root().join("sequence.json"),
        r#"{"sequence": [2, 3]}"#,
    )
    .unwrap();
    sb
}

fn backup_gets(sb: &Sandbox, num: &str, email: &str) -> usize {
    sb.keychain.calls_for(Op::Get, &backup_account(num, email))
}

// ---- namespace ------------------------------------------------------------------------------------------

#[test]
fn read_uses_the_haryx_backup_service() {
    let sb = Sandbox::macos();
    sb.keychain.put_for(
        HARYX_BACKUP_SERVICE,
        "account-1-user@example.com",
        "fake-token",
    );
    sb.keychain.put_for(
        CSWAP_BACKUP_SERVICE,
        "account-1-user@example.com",
        "cswap-token",
    );
    assert_eq!(
        sb.store()
            .read_account_credentials("1", "user@example.com", false),
        "fake-token"
    );
    assert_eq!(
        sb.keychain.all_calls(),
        vec![(
            Op::Get,
            "haryx".to_string(),
            "account-1-user@example.com".to_string()
        )]
    );
}

#[test]
fn a_cswap_host_reads_the_cswap_service() {
    let sb = Sandbox::macos();
    sb.keychain.put_for(
        CSWAP_BACKUP_SERVICE,
        "account-1-user@example.com",
        "cswap-token",
    );
    let ns = BackupNamespace::cswap(&sb.home().join(".claude-swap-backup"));
    let host = StoreHost {
        keychain_account: ACCOUNT.into(),
        ..StoreHost::new(&ns, sb.env.clone())
    };
    let store = CredentialStore::with_keychain(host, sb.keychain.clone(), sb.tunables);
    assert_eq!(
        store.read_account_credentials("1", "user@example.com", false),
        "cswap-token"
    );
}

#[test]
fn write_uses_the_haryx_backup_service() {
    let sb = Sandbox::macos();
    sb.store()
        .write_account_credentials("2", "alice@example.com", "secret-token")
        .unwrap();
    let sets: Vec<_> = sb
        .keychain
        .all_calls()
        .into_iter()
        .filter(|(op, _, _)| *op == Op::Set)
        .collect();
    assert_eq!(
        sets,
        vec![(
            Op::Set,
            "haryx".to_string(),
            "account-2-alice@example.com".to_string()
        )]
    );
}

#[test]
fn write_retains_prev_generation_in_keychain_not_a_file() {
    let sb = Sandbox::macos();
    sb.keychain.put_for(
        HARYX_BACKUP_SERVICE,
        "account-2-alice@example.com",
        "old-generation",
    );
    let store = sb.store();
    store
        .write_account_credentials("2", "alice@example.com", "secret-token")
        .unwrap();
    let sets: Vec<_> = sb
        .keychain
        .all_calls()
        .into_iter()
        .filter(|(op, _, _)| *op == Op::Set)
        .map(|(_, _, a)| a)
        .collect();
    assert_eq!(
        sets,
        vec![
            "account-2-alice@example.com.prev",
            "account-2-alice@example.com"
        ]
    );
    assert_eq!(
        sb.keychain
            .get(HARYX_BACKUP_SERVICE, "account-2-alice@example.com.prev")
            .as_deref(),
        Some("old-generation")
    );
    assert!(!store.prev_backup_path("2", "alice@example.com").exists());
    assert_eq!(
        store.read_previous_backup("2", "alice@example.com"),
        "old-generation"
    );
}

#[test]
fn delete_sweeps_both_keys_and_the_legacy_alias() {
    let sb = Sandbox::macos();
    sb.store()
        .delete_account_credentials("3", "bob@example.com");
    let deletes: Vec<_> = sb
        .keychain
        .all_calls()
        .into_iter()
        .filter(|(op, _, _)| *op == Op::Delete)
        .map(|(_, s, a)| (s, a))
        .collect();
    let haryx = |a: &str| ("haryx".to_string(), a.to_string());
    assert_eq!(
        deletes,
        vec![
            haryx("account-3-bob@example.com"),
            haryx("account-3-bob@example.com.prev"),
            haryx("account-None-bob@example.com"),
            haryx("account-None-bob@example.com.prev"),
        ]
    );
}

// ---- files and .enc-wins --------------------------------------------------------------------------------

#[test]
fn linux_backups_are_base64_enc_files() {
    let sb = Sandbox::linux();
    let store = sb.store();
    store
        .write_account_credentials("1", "a@b.c", &creds("one"))
        .unwrap();
    let path = store.backup_enc_path("1", "a@b.c");
    assert_eq!(path, sb.store_root().join("credentials/.creds-1-a@b.c.enc"));
    let raw = std::fs::read_to_string(&path).unwrap();
    // The file form follows the OS haryx runs on, not the simulated host: Windows seals with DPAPI.
    #[cfg(not(windows))]
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .unwrap(),
        creds("one").as_bytes()
    );
    #[cfg(windows)]
    assert!(
        raw.starts_with(crate::credentials::sealed::DPAPI_PREFIX),
        "{raw}"
    );
    assert_eq!(
        store.read_account_credentials_ex("1", "a@b.c", false),
        (creds("one"), false)
    );

    store
        .write_account_credentials("1", "a@b.c", &creds("two"))
        .unwrap();
    assert_eq!(store.read_previous_backup("1", "a@b.c"), creds("one"));
    assert!(store.prev_backup_path("1", "a@b.c").exists());
    assert!(sb.keychain.all_calls().is_empty());
}

#[test]
fn an_enc_file_wins_over_the_keychain_copy() {
    let sb = seeded();
    let store = sb.store();
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(creds("file"));
    std::fs::write(store.backup_enc_path("2", EMAIL2), format!("{encoded}\n")).unwrap();
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, false),
        creds("file")
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 0);
}

#[test]
fn a_corrupt_enc_falls_through_to_the_keychain() {
    let sb = seeded();
    let store = sb.store();
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    for junk in ["!!!!", "", "\u{ff}"] {
        std::fs::write(store.backup_enc_path("2", EMAIL2), junk).unwrap();
        assert_eq!(
            store.read_account_credentials_ex("2", EMAIL2, false),
            (creds("backup2"), false),
            "{junk:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_enc_is_unreadable_not_absent() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::linux();
    let store = sb.store();
    store.write_account_credentials("1", "a@b.c", "x").unwrap();
    let path = store.backup_enc_path("1", "a@b.c");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        return; // root
    }
    assert_eq!(
        store.read_account_credentials_ex("1", "a@b.c", false),
        (String::new(), true)
    );
}

#[test]
fn a_keychain_write_reconciles_a_leftover_enc_away() {
    let sb = seeded();
    let store = sb.store();
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    std::fs::write(store.backup_enc_path("2", EMAIL2), "c3RhbGU=").unwrap();
    store
        .write_account_credentials("2", EMAIL2, &creds("fresh"))
        .unwrap();
    assert!(!store.backup_enc_path("2", EMAIL2).exists());
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, false),
        creds("fresh")
    );
}

#[test]
fn a_failed_keychain_write_falls_back_to_enc_and_drops_the_stale_item() {
    let sb = seeded();
    sb.keychain.fail_when(|op, _, _| op == Op::Set);
    let store = sb.store();
    store
        .write_account_credentials("2", EMAIL2, &creds("fresh"))
        .unwrap();
    assert!(store.backup_enc_path("2", EMAIL2).exists());
    // The routing flipped to file mode; the quiet delete still ran (and succeeded here).
    assert_eq!(
        sb.keychain
            .get(HARYX_BACKUP_SERVICE, &backup_account("2", EMAIL2)),
        None
    );
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, false),
        creds("fresh")
    );
}

#[test]
fn an_unreadable_current_backup_is_not_retained_and_the_write_proceeds() {
    let sb = seeded();
    sb.keychain.fail_when(|op, _, _| op == Op::Get);
    let store = sb.store();
    store
        .write_account_credentials("2", EMAIL2, &creds("fresh"))
        .unwrap();
    assert!(!store.prev_backup_path("2", EMAIL2).exists());
    assert!(
        sb.keychain
            .get(
                HARYX_BACKUP_SERVICE,
                &format!("{}.prev", backup_account("2", EMAIL2))
            )
            .is_none()
    );
}

// ---- strict delete --------------------------------------------------------------------------------------

#[test]
fn strict_delete_succeeds_when_both_backends_are_clear() {
    let sb = seeded();
    let store = sb.store();
    store
        .delete_account_credentials_strict("2", EMAIL2)
        .unwrap();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, false),
        (String::new(), false)
    );
}

#[test]
fn strict_delete_fails_closed_on_a_locked_keychain() {
    let sb = seeded();
    sb.keychain.fail_when(|op, _, _| op == Op::Delete);
    let err = sb
        .store()
        .delete_account_credentials_strict("2", EMAIL2)
        .unwrap_err();
    assert!(err.is_credential(), "{err:?}");
    assert!(err.to_string().contains("aborting before commit"));
}

#[test]
fn strict_delete_fails_closed_when_the_read_back_is_unreadable() {
    let sb = seeded();
    sb.keychain.fail_when(|op, _, _| op == Op::Get);
    let err = sb
        .store()
        .delete_account_credentials_strict("2", EMAIL2)
        .unwrap_err();
    assert!(err.is_credential());
}

// ---- cooldown -------------------------------------------------------------------------------------------

#[test]
fn a_failed_backup_read_backs_off_the_next_one() {
    let sb = seeded();
    let store = sb.store();
    sb.keychain.fail_all();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, false),
        (String::new(), true)
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 1);

    sb.keychain.heal();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, false),
        (String::new(), true)
    );
    assert_eq!(
        backup_gets(&sb, "2", EMAIL2),
        1,
        "a read inside the cooldown spawned `security`"
    );

    store.expire_cooldown();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, false),
        (creds("backup2"), false)
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 2);
}

#[test]
fn the_active_reads_failure_backs_off_backup_reads_too() {
    let sb = seeded();
    let store = sb.store();
    sb.keychain
        .fail_when(|_, service, _| service == CLAUDE_CODE_KEYCHAIN_SERVICE);
    assert!(store.read_active_credentials().keychain_unavailable);
    sb.keychain.heal();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, false),
        (String::new(), true)
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 0);
}

#[test]
fn pinned_file_mode_does_not_block_backup_reads() {
    for residual_cleared in [true, false] {
        let sb = seeded();
        let store = sb.store();
        store.pin_file_mode(residual_cleared);
        assert!(!store.cooldown_armed());
        assert_eq!(
            store.read_account_credentials("2", EMAIL2, false),
            creds("backup2")
        );
        assert_eq!(backup_gets(&sb, "2", EMAIL2), 1);
    }
}

// ---- display read cache ---------------------------------------------------------------------------------

#[test]
fn cached_reads_spawn_once_per_slot_until_the_ttl() {
    let mut sb = seeded();
    let store = sb.store();
    for _ in 0..3 {
        assert_eq!(
            store.read_account_credentials("2", EMAIL2, true),
            creds("backup2")
        );
    }
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 1);

    sb.tunables.backup_read_cache_ttl = Duration::ZERO;
    let expired = sb.store();
    expired.read_account_credentials("2", EMAIL2, true);
    expired.read_account_credentials("2", EMAIL2, true);
    assert_eq!(
        backup_gets(&sb, "2", EMAIL2),
        3,
        "an expired entry must be re-read"
    );
}

#[test]
fn the_default_read_always_spawns() {
    let sb = seeded();
    let store = sb.store();
    store.read_account_credentials("2", EMAIL2, true);
    store.read_account_credentials("2", EMAIL2, false);
    store.read_account_credentials_ex("2", EMAIL2, false);
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 3);
    store.read_account_credentials("2", EMAIL2, true);
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 3);
}

#[test]
fn own_write_and_delete_drop_the_cache() {
    let sb = seeded();
    let store = sb.store();
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, true),
        creds("backup2")
    );
    store
        .write_account_credentials("2", EMAIL2, &creds("rotated"))
        .unwrap();
    let before = backup_gets(&sb, "2", EMAIL2);
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, true),
        creds("rotated")
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), before + 1);
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, true),
        creds("rotated")
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), before + 1);

    store.delete_account_credentials("2", EMAIL2);
    let before = backup_gets(&sb, "2", EMAIL2);
    assert_eq!(store.read_account_credentials("2", EMAIL2, true), "");
    assert_eq!(backup_gets(&sb, "2", EMAIL2), before + 1);
}

#[test]
fn a_registry_change_drops_the_cache() {
    let sb = seeded();
    let store = sb.store();
    store.read_account_credentials("2", EMAIL2, true);
    store.read_account_credentials("2", EMAIL2, true);
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 1);

    // Same length, rewritten atomically (new inode).
    let seq = sb.store_root().join("sequence.json");
    let tmp = sb.store_root().join("sequence.json.tmp");
    std::fs::write(&tmp, r#"{"sequence": [3, 2]}"#).unwrap();
    std::fs::rename(&tmp, &seq).unwrap();
    store.read_account_credentials("2", EMAIL2, true);
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 2);
}

#[test]
fn a_failed_read_is_never_cached() {
    let sb = seeded();
    let store = sb.store();
    sb.keychain.fail_all();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, true),
        (String::new(), true)
    );
    sb.keychain.heal();
    store.expire_cooldown();
    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, true),
        (creds("backup2"), false)
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 2);
}

#[test]
fn a_cached_hit_is_served_during_the_cooldown() {
    let sb = seeded();
    let store = sb.store();
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, true),
        creds("backup2")
    );
    sb.keychain.fail_all();
    assert!(
        store
            .kc_read_backup("9", "nobody@example.com", false)
            .is_err()
    ); // arms the cooldown
    sb.keychain.heal();

    assert_eq!(
        store.read_account_credentials_ex("2", EMAIL2, true),
        (creds("backup2"), false)
    );
    assert_eq!(backup_gets(&sb, "2", EMAIL2), 1);
    assert_eq!(
        store.read_account_credentials_ex("3", EMAIL3, true),
        (String::new(), true)
    );
    assert_eq!(backup_gets(&sb, "3", EMAIL3), 0);
}

#[test]
fn a_drop_during_the_fill_wins() {
    let sb = seeded();
    let store = Arc::new(sb.store());
    let weak = Arc::downgrade(&store);
    sb.keychain.on_op(move |op, _, _| {
        if op == Op::Get
            && let Some(store) = weak.upgrade()
        {
            store.drop_backup_read_cache(); // lands while `security` runs
        }
    });
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, true),
        creds("backup2")
    );
    sb.keychain.clear_hook();
    store.read_account_credentials("2", EMAIL2, true);
    assert_eq!(
        backup_gets(&sb, "2", EMAIL2),
        2,
        "a fill that raced a drop was kept"
    );
}

#[test]
fn a_write_landing_after_a_racing_fill_is_not_shadowed() {
    let sb = seeded();
    let store = Arc::new(sb.store());
    let weak = Arc::downgrade(&store);
    sb.keychain.on_op(move |op, _, account| {
        if op == Op::Set
            && account == "account-2-user2@example.com"
            && let Some(store) = weak.upgrade()
        {
            // A display read slips in before the write lands and caches the old generation.
            assert_eq!(
                store.read_account_credentials("2", EMAIL2, true),
                creds("backup2")
            );
        }
    });
    store
        .kc_write_backup("2", EMAIL2, &creds("rotated"))
        .unwrap();
    sb.keychain.clear_hook();
    assert_eq!(
        store.read_account_credentials("2", EMAIL2, true),
        creds("rotated")
    );
}

// ---- unclaimed stash ------------------------------------------------------------------------------------

fn context() -> Map<String, Value> {
    json!({"reason": "unclaimed", "outgoingSlot": "2"})
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn a_stash_round_trips_and_is_removed() {
    let sb = Sandbox::macos();
    let store = sb.store();
    let id = store
        .write_unclaimed_credential(&creds("orphan"), &context())
        .unwrap();
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 3, "{id}");
    assert_eq!(parts[0].len(), 15);
    assert_eq!(parts[1].len(), 12);
    assert_eq!(parts[2].len(), 6);

    let listed = store.list_unclaimed_credentials();
    let row = listed[&id].as_object().unwrap();
    assert_eq!(row["reason"], "unclaimed");
    assert!(row["createdAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        store.read_unclaimed_credential(&id),
        (creds("orphan"), false)
    );
    assert!(
        sb.keychain.all_calls().is_empty(),
        "the stash never uses the Keychain"
    );

    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(store.credentials_dir().join(".unclaimed-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["schemaVersion"], 1);

    store.remove_unclaimed_credential(&id).unwrap();
    assert!(store.list_unclaimed_credentials().is_empty());
    assert_eq!(store.read_unclaimed_credential(&id), (String::new(), false));
}

#[test]
fn identical_bytes_get_distinct_ids() {
    let sb = Sandbox::linux();
    let store = sb.store();
    let a = store
        .write_unclaimed_credential("same", &Map::new())
        .unwrap();
    let b = store
        .write_unclaimed_credential("same", &Map::new())
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(store.list_unclaimed_credentials().len(), 2);
}

#[test]
fn orphan_entry_files_are_listed_without_metadata() {
    let sb = Sandbox::linux();
    let store = sb.store();
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    std::fs::write(
        store.credentials_dir().join(".unclaimed-orphan1.enc"),
        "eA==",
    )
    .unwrap();
    assert!(store.stash_entry_files_exist());
    let listed = store.list_unclaimed_credentials();
    assert_eq!(listed["orphan1"], json!({"createdAt": null}));
    assert_eq!(
        store.read_unclaimed_credential("orphan1"),
        ("x".to_string(), false)
    );
}

#[test]
fn manifest_verdicts_distinguish_absent_corrupt_and_structural() {
    let sb = Sandbox::linux();
    let store = sb.store();
    assert_eq!(store.read_stash_manifest_ex().1, ManifestVerdict::Ok);
    assert!(
        !store.stash_entry_files_exist(),
        "no credentials dir means nothing stashed"
    );
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    let manifest = store.credentials_dir().join(".unclaimed-manifest.json");
    for (content, verdict) in [
        (&b"{not json"[..], ManifestVerdict::Corrupt),
        (&b"\xff\xfe"[..], ManifestVerdict::Corrupt),
        (&br#"{"schemaVersion": 1}"#[..], ManifestVerdict::Corrupt),
        (&br#"{"entries": []}"#[..], ManifestVerdict::Corrupt),
        (&br#"[1]"#[..], ManifestVerdict::Corrupt),
        (&br#"{"entries": {"a": {}}}"#[..], ManifestVerdict::Ok),
    ] {
        std::fs::write(&manifest, content).unwrap();
        assert_eq!(store.read_stash_manifest_ex().1, verdict, "{content:?}");
    }
}

#[test]
fn a_corrupt_manifest_is_set_aside_on_the_next_write() {
    let sb = Sandbox::linux();
    let store = sb.store();
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    std::fs::write(
        store.credentials_dir().join(".unclaimed-manifest.json"),
        "{torn",
    )
    .unwrap();
    let id = store.write_unclaimed_credential("x", &Map::new()).unwrap();
    let names: Vec<String> = std::fs::read_dir(store.credentials_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with(".unclaimed-manifest.json.corrupt-")),
        "{names:?}"
    );
    assert_eq!(store.read_stash_manifest_ex().1, ManifestVerdict::Ok);
    assert!(store.read_stash_manifest().contains_key(&id));
}

#[cfg(unix)]
#[test]
fn an_unreadable_manifest_refuses_a_rewrite() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::linux();
    let store = sb.store();
    store
        .write_unclaimed_credential("first", &Map::new())
        .unwrap();
    let manifest = store.credentials_dir().join(".unclaimed-manifest.json");
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&manifest).is_ok() {
        return; // root
    }
    assert_eq!(
        store.read_stash_manifest_ex().1,
        ManifestVerdict::Unreadable
    );
    let err = store
        .write_unclaimed_credential("second", &Map::new())
        .unwrap_err();
    assert!(matches!(err, crate::Error::CredentialRead(_)), "{err:?}");
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        store.read_stash_manifest().len(),
        1,
        "the healthy manifest was not orphaned"
    );
}

#[test]
fn a_corrupt_entry_reads_as_empty_not_unreadable() {
    let sb = Sandbox::linux();
    let store = sb.store();
    std::fs::create_dir_all(store.credentials_dir()).unwrap();
    std::fs::write(store.credentials_dir().join(".unclaimed-bad.enc"), "!!!!").unwrap();
    assert_eq!(
        store.read_unclaimed_credential("bad"),
        (String::new(), false)
    );
}
