//! `haryx run` end to end with a fake `claude` on PATH (answers `auth status`, echoes its environment).

// TODO(windows): these tests drive fake `#!/bin/sh` binaries; port them to `.cmd` fakes.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use serde_json::json;

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn run_launches_claude_in_a_session_profile_with_shared_items() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let bin = home.join("bin");
    std::fs::create_dir_all(home.join(".claude/skills")).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    write_exec(&home.join("security"), "#!/bin/sh\nexit 44\n");
    write_exec(
        &bin.join("claude"),
        "#!/bin/sh\nif [ \"$1 $2\" = \"auth status\" ]; then\n  email=$(sed 's/.*\"emailAddress\": *\"\\([^\"]*\\)\".*/\\1/' \"$CLAUDE_CONFIG_DIR/.claude.json\" | tr -d '\\n')\n  printf '{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\"email\":\"%s\"}' \"$email\"; exit 0\nfi\necho \"RAN config=$CLAUDE_CONFIG_DIR args=$* key=${ANTHROPIC_API_KEY:-unset}\"\n",
    );
    std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let haryx = |argv: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_haryx"))
            .args(argv)
            .env("HOME", home)
            .env("HARYX_HOME", home.join("store"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("HARYX_SECURITY_BIN", home.join("security"))
            .env("HARYX_API_BASE", "http://127.0.0.1:9")
            .env("PATH", &path)
            .env("NO_COLOR", "1")
            .env("ANTHROPIC_API_KEY", "sk-should-be-scrubbed")
            .output()
            .unwrap()
    };
    let login = |email: &str, token: &str| {
        // One line, so the fake claude's sed can read the email.
        std::fs::write(
            home.join(".claude.json"),
            json!({"oauthAccount": {"emailAddress": email, "accountUuid": format!("u-{token}"),
                "organizationUuid": "", "organizationName": ""}, "mcpServers": {"m": {"command": "x"}}})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            home.join(".claude/.credentials.json"),
            json!({"claudeAiOauth": {"accessToken": token, "refreshToken": format!("rt-{token}"),
                "expiresAt": 9_999_999_999_000_i64}})
            .to_string(),
        )
        .unwrap();
    };
    login("a@example.com", "at-a");
    assert!(haryx(&["add"]).status.success());
    login("b@example.com", "at-b");
    assert!(haryx(&["add"]).status.success());

    let out = haryx(&["run", "1", "--", "--resume"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stdout.starts_with("RAN config="),
        "stdout belongs to claude alone: {stdout}"
    );
    assert!(
        stdout.contains("sessions/1-a_example.com args=--resume key=unset"),
        "{stdout}"
    );
    assert!(
        stderr.contains("Launching Account-1 (a@example.com) [session mode]"),
        "{stderr}"
    );

    let profile = home.join("store/sessions/1-a_example.com");
    for item in ["settings.json", "skills", "projects", "history.jsonl"] {
        assert!(
            std::fs::symlink_metadata(profile.join(item))
                .unwrap()
                .file_type()
                .is_symlink(),
            "{item}"
        );
    }
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(profile.join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(config["oauthAccount"]["emailAddress"], "a@example.com");
    assert_eq!(config["mcpServers"], json!({"m": {"command": "x"}}));

    let direct = haryx(&["run", "2"]);
    assert!(String::from_utf8_lossy(&direct.stdout).starts_with("RAN config= "));
    assert!(String::from_utf8_lossy(&direct.stderr).contains("already the active default login"));
}
