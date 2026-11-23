//! Account-management commands end to end (alias, threshold, priority, map/unmap, disable/enable,
//! unclaimed, swap/move errors). Each run gets a temp HOME and store and a fake `security` binary, so
//! the real store and Keychain are never touched. The switcher logic itself is unit-tested in core.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

struct Sandbox {
    dir: TempDir,
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Sandbox {
            dir: tempfile::tempdir().unwrap(),
        };
        // A `security` that knows no items: every lookup answers "not found" (exit 44).
        let fake = sandbox.dir.path().join("security");
        std::fs::write(&fake, "#!/bin/sh\nexit 44\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(sandbox.store()).unwrap();
        sandbox.write_roster(json!({
            "activeAccountNumber": 1,
            "lastUpdated": "2024-01-01T00:00:00Z",
            "sequence": [1, 2],
            "accounts": {
                "1": {"email": "account1@example.com", "uuid": "uuid-1", "added": "2024-01-01T00:00:00Z"},
                "2": {"email": "account2@example.com", "uuid": "uuid-2", "added": "2024-01-02T00:00:00Z"},
            },
        }));
        sandbox
    }

    fn store(&self) -> PathBuf {
        self.dir.path().join("store")
    }

    fn write_roster(&self, data: Value) {
        std::fs::write(
            self.store().join("sequence.json"),
            serde_json::to_string_pretty(&data).unwrap(),
        )
        .unwrap();
    }

    fn roster(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.store().join("sequence.json")).unwrap())
            .unwrap()
    }

    fn run(&self, argv: &[&str]) -> Run {
        let output = Command::new(env!("CARGO_BIN_EXE_haryx"))
            .args(argv)
            .current_dir(self.dir.path())
            .env("HOME", self.dir.path())
            .env("HARYX_HOME", self.store())
            .env("CLAUDE_CONFIG_DIR", self.dir.path().join(".claude"))
            .env("HARYX_SECURITY_BIN", self.dir.path().join("security"))
            .env("NO_COLOR", "1")
            .env("HARYX_API_BASE", "http://127.0.0.1:9")
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

#[test]
fn alias_set_list_and_unset() {
    let s = Sandbox::new();
    let set = s.run(&["alias", "2", "Dev"]);
    assert_eq!(set.code, 0, "{}", set.err);
    assert_eq!(set.out, "Set alias 'dev' for Account 2\n");
    assert_eq!(s.roster()["accounts"]["2"]["alias"], "dev");

    let list = s.run(&["alias"]);
    assert_eq!(list.out, "Aliases:\n  2: dev (account2@example.com)\n");

    let unset = s.run(&["alias", "dev", "--unset"]);
    assert_eq!(unset.code, 0, "{}", unset.err);
    assert_eq!(unset.out, "Removed alias for Account 2\n");
    assert_eq!(s.run(&["alias"]).out, "No aliases set\n");
}

#[test]
fn alias_rejects_numeric_name_without_touching_the_roster() {
    let s = Sandbox::new();
    let before = s.roster();
    let run = s.run(&["alias", "1", "42"]);
    assert_eq!(run.code, 1);
    assert!(
        run.err
            .starts_with("Error: alias '42' cannot be purely numeric"),
        "{}",
        run.err
    );
    assert_eq!(s.roster(), before);
}

#[test]
fn unknown_account_is_an_error() {
    let s = Sandbox::new();
    let run = s.run(&["alias", "9", "x"]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("Error: "), "{}", run.err);
}

#[test]
fn threshold_table_and_override() {
    let s = Sandbox::new();
    let table = s.run(&["threshold"]);
    assert_eq!(table.code, 0, "{}", table.err);
    assert!(
        table
            .out
            .starts_with("Switch-away thresholds:\n  global default  ")
    );
    assert!(table.out.ends_with("  No per-account overrides set\n"));

    let set = s.run(&["threshold", "2", "85"]);
    assert_eq!(set.code, 0, "{}", set.err);
    assert!(s.run(&["threshold"]).out.ends_with("  Account-2  85%\n"));

    let unset = s.run(&["threshold", "2", "--unset"]);
    assert_eq!(unset.code, 0, "{}", unset.err);
    assert!(
        s.run(&["threshold"])
            .out
            .ends_with("No per-account overrides set\n")
    );
}

#[test]
fn threshold_out_of_range_names_the_range() {
    let s = Sandbox::new();
    let before = s.roster();
    let run = s.run(&["threshold", "1", "150"]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("between 50 and 99.9"), "{}", run.err);
    assert_eq!(s.roster(), before);
}

#[test]
fn priority_list_and_set() {
    let s = Sandbox::new();
    assert_eq!(s.run(&["priority", "2", "250"]).code, 0);
    let list = s.run(&["priority"]);
    assert_eq!(list.code, 0, "{}", list.err);
    assert!(
        list.out
            .starts_with("Account priorities (higher first; default 0):\n")
    );
    assert!(
        list.out
            .contains("  Account-2  account2@example.com  250\n"),
        "{}",
        list.out
    );
    assert!(
        list.out
            .ends_with("Used only by: haryx auto --strategy priority-consume-first\n")
    );
    assert_eq!(s.run(&["priority", "2", "abc"]).code, 1);
}

#[test]
fn map_and_unmap_a_directory() {
    let s = Sandbox::new();
    let project = s.dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.to_str().unwrap();

    let map = s.run(&["map", "2", path]);
    assert_eq!(map.code, 0, "{}", map.err);
    assert!(map.out.starts_with("Mapped "), "{}", map.out);
    assert!(
        map.out.ends_with("→ Account-2 (account2@example.com)\n"),
        "{}",
        map.out
    );

    let remap = s.run(&["map", "1", path]);
    assert!(
        remap.out.contains("(was account2@example.com)"),
        "{}",
        remap.out
    );

    let unmap = s.run(&["unmap", path]);
    assert!(unmap.out.starts_with("Unmapped "), "{}", unmap.out);
    assert!(s.run(&["unmap", path]).out.starts_with("No mapping for "));
}

#[test]
fn disable_and_enable_flag_the_account() {
    let s = Sandbox::new();
    let off = s.run(&["disable", "2"]);
    assert_eq!(off.code, 0, "{}", off.err);
    assert_eq!(s.roster()["accounts"]["2"]["disabled"], true);
    let on = s.run(&["--enable-account", "2"]);
    assert_eq!(on.code, 0, "{}", on.err);
    assert!(s.roster()["accounts"]["2"].get("disabled").is_none());
}

#[test]
fn unclaimed_empty_and_unknown_purge() {
    let s = Sandbox::new();
    assert_eq!(
        s.run(&["unclaimed"]).out,
        "No unclaimed credential entries\n"
    );
    let purge = s.run(&["unclaimed", "--purge", "nope"]);
    assert_eq!(purge.code, 1);
    assert_eq!(purge.err.trim_end(), "Error: no unclaimed entry nope");
}

#[test]
fn swap_same_account_is_rejected() {
    let s = Sandbox::new();
    let run = s.run(&["swap", "1", "1"]);
    assert_eq!(run.code, 1);
    assert!(run.err.starts_with("Error: "), "{}", run.err);
}

#[test]
fn list_json_matches_the_contract_shape() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.dir.path().join(".claude")).unwrap();
    std::fs::write(
        s.dir.path().join(".claude/.claude.json"),
        json!({"oauthAccount": {"emailAddress": "account2@example.com", "organizationUuid": ""}})
            .to_string(),
    )
    .unwrap();
    s.run(&["alias", "2", "work"]);
    let run = s.run(&["list", "--json"]);
    assert_eq!(run.code, 0, "{}", run.err);
    let payload: Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(payload["schemaVersion"], 1);
    assert_eq!(payload["activeAccountNumber"], 2);
    let rows = payload["accounts"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["alias"], "work");
    assert_eq!(rows[1]["active"], true);
    assert_eq!(rows[0]["usageStatus"], "no_credentials");
    let keys: Vec<&str> = rows[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "number",
            "email",
            "organizationName",
            "organizationUuid",
            "isOrganization",
            "active",
            "usageStatus",
            "usage"
        ]
    );
}

#[test]
fn list_human_view_and_status() {
    let s = Sandbox::new();
    let run = s.run(&["list"]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(
        run.out
            .starts_with("Accounts:\n  1: account1@example.com\n"),
        "{}",
        run.out
    );
    assert!(
        run.out.contains("no credentials") || run.out.contains("usage unavailable"),
        "{}",
        run.out
    );

    let status = s.run(&["status"]);
    assert_eq!(status.code, 0, "{}", status.err);
    assert_eq!(status.out, "Status: No active Claude account\n");
    let json = s.run(&["status", "--json"]);
    assert_eq!(
        json.out.trim(),
        "{\n  \"schemaVersion\": 1,\n  \"active\": null\n}"
    );
}

#[test]
fn list_json_without_a_store_is_empty() {
    let s = Sandbox::new();
    std::fs::remove_file(s.store().join("sequence.json")).unwrap();
    let run = s.run(&["list", "--json"]);
    let payload: Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(
        payload,
        json!({"schemaVersion": 1, "activeAccountNumber": null, "accounts": []})
    );
}

fn login(s: &Sandbox, email: &str, uuid: &str, token: &str) {
    let dir = s.dir.path().join(".claude");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        json!({"oauthAccount": {"emailAddress": email, "accountUuid": uuid, "organizationUuid": "",
            "organizationName": ""}, "theme": "dark"})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join(".credentials.json"),
        json!({"claudeAiOauth": {"accessToken": token, "refreshToken": format!("rt-{token}"),
            "expiresAt": 9_999_999_999_000_i64}})
        .to_string(),
    )
    .unwrap();
}

fn live_token(s: &Sandbox) -> String {
    let creds: Value = serde_json::from_str(
        &std::fs::read_to_string(s.dir.path().join(".claude/.credentials.json")).unwrap(),
    )
    .unwrap();
    creds["claudeAiOauth"]["accessToken"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn add_then_switch_round_trip() {
    let s = Sandbox::new();
    std::fs::remove_file(s.store().join("sequence.json")).unwrap();
    login(&s, "a@example.com", "u-a", "at-a");
    assert_eq!(s.run(&["add"]).code, 0);
    login(&s, "b@example.com", "u-b", "at-b");
    assert_eq!(s.run(&["add"]).code, 0);

    let rotate = s.run(&["switch"]);
    assert_eq!(rotate.code, 0, "{}", rotate.err);
    assert!(
        rotate
            .out
            .starts_with("Switched to Account-1 (a@example.com)"),
        "{}",
        rotate.out
    );
    assert_eq!(live_token(&s), "at-a");
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(s.dir.path().join(".claude/.claude.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["oauthAccount"]["emailAddress"], "a@example.com");
    assert_eq!(
        config["theme"], "dark",
        "the rest of .claude.json is left alone"
    );

    let to_b = s.run(&["switch", "2", "--json"]);
    let payload: Value = serde_json::from_str(&to_b.out).unwrap();
    assert_eq!(payload["switched"], true);
    assert_eq!(
        payload["from"],
        json!({"number": 1, "email": "a@example.com"})
    );
    assert_eq!(
        payload["to"],
        json!({"number": 2, "email": "b@example.com"})
    );
    assert_eq!(live_token(&s), "at-b");

    let again = s.run(&["switch", "2", "--json"]);
    let payload: Value = serde_json::from_str(&again.out).unwrap();
    assert_eq!(payload["reason"], "already-active");
}

#[test]
fn switch_with_one_account_says_so() {
    let s = Sandbox::new();
    std::fs::remove_file(s.store().join("sequence.json")).unwrap();
    login(&s, "a@example.com", "u-a", "at-a");
    s.run(&["add"]);
    let run = s.run(&["switch", "--json"]);
    let payload: Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(payload["reason"], "only-one-account");
}

#[test]
fn purge_removes_the_store_but_not_the_live_login() {
    let s = Sandbox::new();
    std::fs::remove_file(s.store().join("sequence.json")).unwrap();
    login(&s, "a@example.com", "u-a", "at-a");
    s.run(&["add"]);
    let output = Command::new(env!("CARGO_BIN_EXE_haryx"))
        .arg("purge")
        .env("HOME", s.dir.path())
        .env("HARYX_HOME", s.store())
        .env("CLAUDE_CONFIG_DIR", s.dir.path().join(".claude"))
        .env("HARYX_SECURITY_BIN", s.dir.path().join("security"))
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"y\n")?;
            child.wait_with_output()
        })
        .unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(out.contains("Purge complete."), "{out}");
    assert!(!s.store().exists());
    assert_eq!(live_token(&s), "at-a");
}

#[test]
fn import_from_cswap_copies_accounts_and_settings() {
    let s = Sandbox::new();
    // A cswap store has the same on-disk layout; build one at cswap's legacy location.
    let cswap = s.dir.path().join(".claude-swap-backup");
    let run_in = |store: &std::path::Path, argv: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_haryx"))
            .args(argv)
            .env("HOME", s.dir.path())
            .env("HARYX_HOME", store)
            .env("CLAUDE_CONFIG_DIR", s.dir.path().join(".claude"))
            .env("HARYX_SECURITY_BIN", s.dir.path().join("security"))
            .env("HARYX_API_BASE", "http://127.0.0.1:9")
            .env("NO_COLOR", "1")
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap()
    };
    login(&s, "a@example.com", "u-a", "at-a");
    run_in(&cswap, &["add", "--alias", "work"]);
    login(&s, "b@example.com", "u-b", "at-b");
    run_in(&cswap, &["add"]);
    std::fs::write(
        cswap.join("settings.json"),
        r#"{"schemaVersion":1,"autoswitch":{"threshold":88}}"#,
    )
    .unwrap();
    // cswap's last reading for b (its slot 2) and b's plan: both carry over by identity.
    let fetched = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        - 600.0;
    std::fs::create_dir_all(cswap.join("cache")).unwrap();
    std::fs::write(
        cswap.join("cache/usage.json"),
        json!({"schemaVersion": 2, "accounts": {"2": {
            "email": "b@example.com", "organizationUuid": "", "fetchedAt": fetched,
            "lastGood": {"five_hour": {"pct": 94.0, "resets_at": null},
                         "seven_day": {"pct": 90.0, "resets_at": null}}}}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        cswap.join("cache/subscription_plans.json"),
        json!({"b@example.com\n": {"plan": {"tier": "default_claude_max_5x",
            "organizationType": "claude_max", "billingType": "stripe_subscription",
            "subscriptionStatus": "active", "planDisplayName": null}, "fetchedAt": fetched}})
        .to_string(),
    )
    .unwrap();

    std::fs::remove_file(s.store().join("sequence.json")).unwrap();
    let out = s.run(&["import", "--from-cswap"]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(
        out.err
            .contains("Done: 2 imported, 0 overwritten, 0 skipped"),
        "{}",
        out.err
    );
    assert_eq!(s.roster()["accounts"]["1"]["alias"], "work");
    assert!(
        out.err
            .contains("Carried over cswap's last usage for 1 account(s) and 1 plan record(s)"),
        "{}",
        out.err
    );
    let listed = s.run(&["list"]);
    assert!(listed.out.contains("· Max 5x"), "{}", listed.out);
    assert!(listed.out.contains("5h:  94%"), "{}", listed.out);
    assert_eq!(
        s.run(&["config", "get", "autoswitch.threshold"]).out.trim(),
        "88"
    );
    assert!(
        cswap.join("sequence.json").exists(),
        "the cswap store is only read"
    );

    // Round trip through an export file: everything already present is skipped.
    let file = s.dir.path().join("backup.json");
    assert_eq!(s.run(&["export", file.to_str().unwrap()]).code, 0);
    let again = s.run(&["import", file.to_str().unwrap()]);
    assert!(
        again
            .err
            .contains("Done: 0 imported, 0 overwritten, 2 skipped"),
        "{}",
        again.err
    );
}

fn events(out: &str) -> Vec<Value> {
    out.lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn auto_once_without_a_live_login_reports_no_active_account() {
    let s = Sandbox::new();
    let run = s.run(&["auto", "--once", "--json"]);
    assert_eq!(run.code, 2, "{}", run.err);
    let events = events(&run.out);
    assert_eq!(events[0]["event"], "poll");
    assert_eq!(events[0]["schemaVersion"], 1);
    assert_eq!(events[0]["active"], Value::Null);
    assert_eq!(events[1]["event"], "no-switch");
    assert_eq!(events[1]["reason"], "no-active-account");
    assert!(
        run.out
            .starts_with("{\"schemaVersion\": 1, \"event\": \"poll\""),
        "Python json.dumps separators: {}",
        run.out
    );
}

#[test]
fn auto_once_counts_unknown_usage_then_fails_over() {
    let s = Sandbox::new();
    std::fs::remove_file(s.store().join("sequence.json")).unwrap();
    login(&s, "a@example.com", "u-a", "at-a");
    assert_eq!(s.run(&["add"]).code, 0);
    login(&s, "b@example.com", "u-b", "at-b");
    assert_eq!(s.run(&["add"]).code, 0);

    for tick in 1..=2 {
        let run = s.run(&["auto", "--once", "--json"]);
        assert_eq!(run.code, 2, "{}", run.err);
        let last = events(&run.out).pop().unwrap();
        assert_eq!(last["reason"], "active-usage-unknown");
        assert_eq!(last["detail"], format!("{tick}/3 before failover"));
    }
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(s.store().join("autoswitch_state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["unhealthyTicks"], 2);
    assert_eq!(state["healthAccount"], "2");

    // Third unknown tick fails over, but no candidate has readable usage either.
    let run = s.run(&["auto", "--once", "--json"]);
    assert_eq!(run.code, 3, "{}", run.out);
    let last = events(&run.out).pop().unwrap();
    assert_eq!(last["reason"], "no-comparison");
    assert_eq!(live_token(&s), "at-b", "nothing switched");
}

#[test]
fn auto_once_human_lines_are_timestamped() {
    let s = Sandbox::new();
    let run = s.run(&["auto", "--once"]);
    assert_eq!(run.code, 2);
    let lines: Vec<&str> = run.out.lines().collect();
    assert!(
        lines[0].ends_with("  poll: no active account"),
        "{}",
        run.out
    );
    assert!(
        lines[1]
            .contains("no switch: no-active-account (log in and run 'haryx --add-account' first)")
    );
}
