//! `haryx ai profile` end to end: two sandboxed machines (temp HOME + HARYX_HOME each), a fake `claude`
//! on PATH that records plugin installs, export on one and import/apply on the other.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

const SECRET: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

struct Machine {
    dir: TempDir,
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Machine {
    fn new() -> Self {
        let m = Machine {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(m.claude()).unwrap();
        std::fs::create_dir_all(m.bin()).unwrap();
        std::fs::write(m.bin().join("npx"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let npx = m.bin().join("npx");
            std::fs::set_permissions(&npx, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let stub = m.bin().join("claude");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\n",
                m.dir.path().join("claude-calls.log").display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        m
    }

    fn home(&self) -> PathBuf {
        // Canonical, so the home text matches the paths written into files (macOS /var -> /private/var).
        std::fs::canonicalize(self.dir.path()).unwrap()
    }

    fn claude(&self) -> PathBuf {
        self.dir.path().join(".claude")
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.claude().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.claude().join(rel)).unwrap()
    }

    fn claude_json(&self) -> Value {
        let text = std::fs::read_to_string(self.home().join(".claude.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("claude-calls.log")).unwrap_or_default()
    }

    fn run_env(&self, argv: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_haryx"));
        cmd.arg("ai")
            .arg("profile")
            .args(argv)
            .env("HOME", self.home())
            .env("HARYX_HOME", self.home().join("store"))
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin().display()))
            .env_remove("XDG_DATA_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("HARYX_PROFILE_PASSPHRASE")
            .stdin(Stdio::null());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let output = cmd.output().unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn ok(&self, argv: &[&str]) -> Run {
        self.ok_env(argv, &[])
    }

    fn ok_env(&self, argv: &[&str], extra: &[(&str, &str)]) -> Run {
        let run = self.run_env(argv, extra);
        assert_eq!(
            run.code, 0,
            "{argv:?}\nstdout: {}\nstderr: {}",
            run.out, run.err
        );
        run
    }

    /// A setup with a skill, a hook script, settings with hooks, an MCP server holding a secret, a
    /// plugin, history and a project folder named after the home.
    fn seed(&self) {
        let home = self.home();
        let home = home.display();
        self.write("skills/review/SKILL.md", "# review\n");
        self.write(
            "hooks/check.sh",
            &format!("#!/bin/sh\ncat {home}/.claude/CLAUDE.md\n"),
        );
        self.write("CLAUDE.md", "be brief\n");
        self.write(
            "settings.json",
            &json!({
                "model": "opus",
                "permissions": {"allow": ["Bash(ls)"]},
                "hooks": {"Stop": [{"hooks": [{"type": "command", "command": format!("{home}/.claude/hooks/check.sh")}]}]}
            })
            .to_string(),
        );
        std::fs::write(
            self.home().join(".claude.json"),
            json!({
                "userID": "machine-a",
                "mcpServers": {"gh": {"command": "npx", "args": ["-y", "gh-mcp"], "env": {"GITHUB_TOKEN": SECRET}}}
            })
            .to_string(),
        )
        .unwrap();
        self.write(
            "plugins/installed_plugins.json",
            &json!({"version": 2, "plugins": {"tool@market": [{"version": "1.0"}]}}).to_string(),
        );
        self.write(
            "plugins/known_marketplaces.json",
            &json!({"market": {"source": {"source": "github", "repo": "o/market"}}}).to_string(),
        );
        self.write("history.jsonl", "{\"display\":\"a\",\"timestamp\":1}\n");
        let slug: String = self
            .home()
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.write(
            &format!("projects/{slug}-app/s1.jsonl"),
            &format!("{{\"cwd\":\"{home}/app\"}}\n"),
        );
        self.write("sessions/123.json", "{}");
    }

    fn slug(&self) -> String {
        self.home()
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }
}

fn export_file(m: &Machine) -> PathBuf {
    m.home().join("out.haryx-profile")
}

#[test]
fn show_lists_live_categories_as_json() {
    let a = Machine::new();
    a.seed();
    let run = a.ok(&["show", "--json"]);
    let doc: Value = serde_json::from_str(&run.out).unwrap();
    let ids: Vec<&str> = doc["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    for want in [
        "skills/review",
        "settings.json#hooks",
        "mcp:gh",
        "plugin:tool@market",
        "projects",
        "CLAUDE.md",
    ] {
        assert!(ids.contains(&want), "{want} missing from {ids:?}");
    }
    assert!(!ids.iter().any(|i| i.starts_with("sessions")));
    let mcp = doc["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "mcp:gh")
        .unwrap();
    assert_eq!(mcp["status"], "secret");
}

#[test]
fn export_redacts_secrets_and_moves_setup_to_another_home() {
    let a = Machine::new();
    a.seed();
    let file = export_file(&a);
    a.ok(&["export", file.to_str().unwrap()]);
    let bytes = std::fs::read(&file).unwrap();
    let text = String::from_utf8_lossy(&flate2_decode(&bytes)).into_owned();
    assert!(!text.contains(SECRET), "secret leaked into the export");
    assert!(
        !text.contains(&a.home().to_string_lossy().into_owned()),
        "home path leaked"
    );

    let b = Machine::new();
    b.ok(&["import", "work", file.to_str().unwrap()]);
    let plan = b.ok(&["apply", "work", "--dry-run", "--json"]);
    let plan: Value = serde_json::from_str(&plan.out).unwrap();
    assert!(
        plan["runs_code"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().starts_with("hook:"))
    );

    // Not at a terminal and installs hooks: refuses without --yes.
    let refused = b.run_env(&["apply", "work"], &[]);
    assert_eq!(refused.code, 1, "{}", refused.out);
    assert!(!b.claude().join("skills/review").exists());

    b.ok(&["apply", "work", "--yes"]);
    assert_eq!(b.read("skills/review/SKILL.md"), "# review\n");
    let script = b.read("hooks/check.sh");
    assert!(
        script.contains(&format!("{}/.claude/CLAUDE.md", b.home().display())),
        "{script}"
    );
    let settings: Value = serde_json::from_str(&b.read("settings.json")).unwrap();
    let hook = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(
        hook.starts_with(&b.home().to_string_lossy().into_owned()),
        "{hook}"
    );
    let mcp = &b.claude_json()["mcpServers"]["gh"];
    assert_eq!(mcp["env"]["GITHUB_TOKEN"], "${HARYX_SECRET_GITHUB_TOKEN}");
    assert!(
        b.claude_json().get("userID").is_none(),
        "claude.json identity is opt-in"
    );
    assert!(
        b.claude()
            .join(format!("projects/{}-app/s1.jsonl", b.slug()))
            .is_file()
    );
    assert!(!b.claude().join("sessions").exists());
    assert!(
        b.calls().contains("plugin marketplace add o/market"),
        "{}",
        b.calls()
    );
    assert!(b.calls().contains("plugin install tool@market"));
}

#[test]
fn env_fills_redacted_secret_on_apply() {
    let a = Machine::new();
    a.seed();
    let file = export_file(&a);
    a.ok(&["export", file.to_str().unwrap()]);
    let b = Machine::new();
    b.ok(&["import", "work", file.to_str().unwrap()]);
    b.ok_env(
        &["apply", "work", "--yes"],
        &[("HARYX_SECRET_GITHUB_TOKEN", "from-env")],
    );
    assert_eq!(
        b.claude_json()["mcpServers"]["gh"]["env"]["GITHUB_TOKEN"],
        "from-env"
    );
}

#[test]
fn encrypted_export_restores_secrets_with_passphrase() {
    let a = Machine::new();
    a.seed();
    let file = export_file(&a);
    let pass = [("HARYX_PROFILE_PASSPHRASE", "correct horse battery")];
    a.ok_env(
        &["export", file.to_str().unwrap(), "--secrets", "encrypt"],
        &pass,
    );
    let b = Machine::new();
    b.ok(&["import", "work", file.to_str().unwrap()]);
    let wrong = b.run_env(
        &["apply", "work", "--yes"],
        &[("HARYX_PROFILE_PASSPHRASE", "wrong passphrase")],
    );
    assert_eq!(wrong.code, 1);
    assert!(wrong.err.contains("Wrong passphrase"), "{}", wrong.err);
    b.ok_env(&["apply", "work", "--yes"], &pass);
    assert_eq!(
        b.claude_json()["mcpServers"]["gh"]["env"]["GITHUB_TOKEN"],
        SECRET
    );
}

#[test]
fn credentials_need_encryption() {
    let a = Machine::new();
    a.seed();
    let file = export_file(&a);
    let run = a.run_env(&["export", file.to_str().unwrap(), "--credentials"], &[]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("--secrets encrypt"), "{}", run.err);
    assert!(!file.exists());
}

#[test]
fn apply_merges_history_and_prune_keeps_it() {
    let a = Machine::new();
    a.seed();
    a.ok(&["save", "work"]);
    // Local changes after the save: an extra skill and a newer prompt.
    a.write("skills/extra/SKILL.md", "# extra\n");
    a.write(
        "history.jsonl",
        "{\"display\":\"a\",\"timestamp\":1}\n{\"display\":\"b\",\"timestamp\":2}\n",
    );
    a.write("projects/other-proj/s.jsonl", "{}\n");

    let dry = a.ok(&["apply", "work", "--prune", "--dry-run", "--json"]);
    let plan: Value = serde_json::from_str(&dry.out).unwrap();
    let action = |id: &str| {
        plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .map(|e| e["action"].as_str().unwrap().to_owned())
    };
    assert_eq!(action("skills/extra").as_deref(), Some("remove"));
    assert_eq!(action("skills/review").as_deref(), Some("same"));
    assert_eq!(
        action("projects/other-proj"),
        None,
        "history is never pruned"
    );

    a.ok(&["apply", "work", "--prune", "--yes"]);
    assert!(!a.claude().join("skills/extra").exists());
    assert!(
        a.read("history.jsonl").contains("\"b\""),
        "local history kept"
    );
    assert!(a.claude().join("projects/other-proj/s.jsonl").is_file());

    a.ok(&["rollback"]);
    assert_eq!(a.read("skills/extra/SKILL.md"), "# extra\n");
}

#[test]
fn list_diff_and_delete() {
    let a = Machine::new();
    a.seed();
    a.ok(&["save", "work"]);
    let list: Value = serde_json::from_str(&a.ok(&["list", "--json"]).out).unwrap();
    assert_eq!(list["profiles"][0]["name"], "work");
    a.write("skills/extra/SKILL.md", "x");
    let diff: Value = serde_json::from_str(&a.ok(&["diff", "work", "--json"]).out).unwrap();
    let extra = diff["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "skills/extra")
        .unwrap();
    assert_eq!(extra["state"], "only-right");
    let refused = a.run_env(&["delete", "work"], &[]);
    assert_eq!(refused.code, 1);
    a.ok(&["delete", "work", "--yes"]);
    assert!(a.ok(&["list", "--json"]).out.contains("\"profiles\": []"));
}

#[test]
fn rejects_reserved_and_bad_names_and_tampered_files() {
    let a = Machine::new();
    a.seed();
    assert_eq!(a.run_env(&["save", "default"], &[]).code, 1);
    assert_eq!(a.run_env(&["save", "../x"], &[]).code, 1);
    let bogus = a.home().join("bogus.haryx-profile");
    std::fs::write(&bogus, b"not a profile").unwrap();
    let run = a.run_env(&["import", "x", bogus.to_str().unwrap()], &[]);
    assert_eq!(run.code, 1);
    assert!(a.ok(&["list", "--json"]).out.contains("\"profiles\": []"));
}

#[test]
fn top_level_profile_is_reserved() {
    let output = Command::new(env!("CARGO_BIN_EXE_haryx"))
        .args(["profile", "list"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("haryx ai profile"));
}

/// The decompressed tar bytes of an export, to search for leaks.
fn flate2_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut child = Command::new("gzip")
        .arg("-dc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    use std::io::Read;
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    child.wait().unwrap();
    out
}

#[test]
fn bundle_copies_plugin_files_and_include_dir_restores_folder() {
    let a = Machine::new();
    a.seed();
    a.write(
        "plugins/cache/market/tool/1.0/plugin.json",
        "{\"name\":\"tool\"}",
    );
    let extra = a.home().join(".claude-mem");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(extra.join("notes.md"), "remember\n").unwrap();
    let file = export_file(&a);
    a.ok(&[
        "save",
        "work",
        "--plugins",
        "bundle",
        "--include-dir",
        extra.to_str().unwrap(),
    ]);
    a.ok(&[
        "export",
        "work",
        file.to_str().unwrap(),
        "--plugins",
        "bundle",
    ]);

    let b = Machine::new();
    b.ok(&["import", "work", file.to_str().unwrap(), "--apply", "--yes"]);
    assert_eq!(
        b.read("plugins/cache/market/tool/1.0/plugin.json"),
        "{\"name\":\"tool\"}"
    );
    let installed: Value = serde_json::from_str(&b.read("plugins/installed_plugins.json")).unwrap();
    assert!(installed["plugins"]["tool@market"].is_array());
    assert!(
        b.calls().is_empty(),
        "bundle installs without the claude CLI: {}",
        b.calls()
    );
    assert_eq!(
        std::fs::read_to_string(b.home().join(".claude-mem/notes.md")).unwrap(),
        "remember\n"
    );
}

#[test]
fn no_history_leaves_conversations_out() {
    let a = Machine::new();
    a.seed();
    let file = export_file(&a);
    a.ok(&["export", file.to_str().unwrap(), "--no-history"]);
    let b = Machine::new();
    b.ok(&["import", "work", file.to_str().unwrap(), "--apply", "--yes"]);
    assert!(b.claude().join("skills/review").exists());
    assert!(!b.claude().join("history.jsonl").exists());
    assert!(!b.claude().join("projects").exists());
}

#[test]
fn settings_that_run_commands_need_confirmation() {
    let a = Machine::new();
    a.write(
        "settings.json",
        &json!({"apiKeyHelper": "/tmp/steal.sh", "theme": "dark"}).to_string(),
    );
    let file = export_file(&a);
    a.ok(&["export", file.to_str().unwrap(), "--no-history"]);
    let b = Machine::new();
    b.ok(&["import", "work", file.to_str().unwrap()]);
    let plan: Value =
        serde_json::from_str(&b.ok(&["apply", "work", "--dry-run", "--json"]).out).unwrap();
    let runs: Vec<&str> = plan["runs_code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        runs.iter().any(|l| l.starts_with("setting apiKeyHelper")),
        "{runs:?}"
    );
    assert!(
        !runs.iter().any(|l| l.contains("theme")),
        "inert keys are not listed"
    );
    let refused = b.run_env(&["apply", "work"], &[]);
    assert_eq!(refused.code, 1);
    assert!(!b.claude().join("settings.json").exists());
}

#[test]
fn included_folder_outside_allowed_places_is_never_written() {
    let a = Machine::new();
    a.seed();
    let extra = a.home().join("notes");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(extra.join("authorized_keys"), "attacker-key\n").unwrap();
    a.ok(&["save", "work", "--include-dir", extra.to_str().unwrap()]);
    // Tamper with the saved profile: point the folder at ~/.ssh.
    let manifest_path = a.home().join("store/profiles/work/manifest.json");
    let mut manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["include_dirs"][0]["origin"] = json!("{{HARYX_HOME}}/.ssh");
    std::fs::write(&manifest_path, manifest.to_string()).unwrap();

    let run = a.ok(&["apply", "work", "--yes"]);
    assert!(
        run.err.contains("never written") || run.out.contains("never written"),
        "{}{}",
        run.out,
        run.err
    );
    assert!(!a.home().join(".ssh").exists());
}

#[test]
fn archive_listing_plain_secrets_is_rejected() {
    let a = Machine::new();
    a.seed();
    a.ok(&["save", "work"]);
    // A hand-made archive whose manifest carries secrets.json.
    let dir = a.home().join("store/profiles/work");
    let mut manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    manifest["files"]["secrets.json"] = json!({"sha256": "0", "size": 1});
    let tar_dir = a.home().join("evil");
    std::fs::create_dir_all(&tar_dir).unwrap();
    std::fs::write(tar_dir.join("manifest.json"), manifest.to_string()).unwrap();
    let file = a.home().join("evil.haryx-profile");
    let status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .args([
            "-czf",
            file.to_str().unwrap(),
            "-C",
            tar_dir.to_str().unwrap(),
            "manifest.json",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let run = a.run_env(&["import", "evil", file.to_str().unwrap()], &[]);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("may not carry"), "{}", run.err);
}
