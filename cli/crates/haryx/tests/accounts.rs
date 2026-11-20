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
