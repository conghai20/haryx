//! `haryx login` end to end with a fake `claude` whose `auth login` "signs in" as `$FAKE_EMAIL`:
//! it writes the login into `$CLAUDE_CONFIG_DIR` (a staging profile) or the default profile.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
[ "$1 $2" = "auth login" ] || { echo "unexpected: $*" >&2; exit 2; }
echo "ARGS $*" >> "$HOME/claude-calls.log"
[ -n "$FAKE_FAIL" ] && exit 1
if [ -n "$CLAUDE_CONFIG_DIR" ]; then
  cfg="$CLAUDE_CONFIG_DIR/.claude.json"; creds="$CLAUDE_CONFIG_DIR/.credentials.json"
else
  mkdir -p "$HOME/.claude"; cfg="$HOME/.claude.json"; creds="$HOME/.claude/.credentials.json"
fi
printf '{"oauthAccount":{"emailAddress":"%s","accountUuid":"u-%s","organizationUuid":"","organizationName":""}}' "$FAKE_EMAIL" "$FAKE_TOKEN" > "$cfg"
printf '{"claudeAiOauth":{"accessToken":"%s","refreshToken":"rt-%s","expiresAt":9999999999000}}' "$FAKE_TOKEN" "$FAKE_TOKEN" > "$creds"
"#;

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Sandbox {
            dir: tempfile::tempdir().unwrap(),
        };
        let bin = sandbox.home().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(sandbox.home().join(".claude")).unwrap();
        write_exec(&sandbox.home().join("security"), "#!/bin/sh\nexit 44\n");
        write_exec(&bin.join("claude"), FAKE_CLAUDE);
        sandbox
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn store(&self) -> PathBuf {
        self.home().join("store")
    }

    /// Put `email` on the default login, as a manual `/login` would.
    fn default_login(&self, email: &str, token: &str) {
        std::fs::write(
            self.home().join(".claude.json"),
            json!({"oauthAccount": {"emailAddress": email, "accountUuid": format!("u-{token}"),
                "organizationUuid": "", "organizationName": ""}})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            self.home().join(".claude/.credentials.json"),
            json!({"claudeAiOauth": {"accessToken": token, "refreshToken": format!("rt-{token}"),
                "expiresAt": 9_999_999_999_000_i64}})
            .to_string(),
        )
        .unwrap();
    }

    fn haryx(&self, argv: &[&str], fake: &[(&str, &str)]) -> Output {
        let path = format!(
            "{}:{}",
            self.home().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_haryx"));
        command
            .args(argv)
            .current_dir(self.home())
            .env("HOME", self.home())
            .env("HARYX_HOME", self.store())
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("HARYX_SECURITY_BIN", self.home().join("security"))
            .env("HARYX_API_BASE", "http://127.0.0.1:9")
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .env_remove("XDG_DATA_HOME");
        for (k, v) in fake {
            command.env(k, v);
        }
        command.output().unwrap()
    }

    fn roster(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.store().join("sequence.json")).unwrap())
            .unwrap()
    }

    /// Every stored file under the store whose contents mention `needle`.
    fn stored_files_with(&self, needle: &str) -> Vec<PathBuf> {
        let mut hits = Vec::new();
        let mut stack = vec![self.store()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if std::fs::read_to_string(&path).is_ok_and(|t| t.contains(needle)) {
                    hits.push(path);
                }
            }
        }
        hits
    }

    fn claude_calls(&self) -> String {
        std::fs::read_to_string(self.home().join("claude-calls.log")).unwrap_or_default()
    }

    fn default_credentials(&self) -> String {
        std::fs::read_to_string(self.home().join(".claude/.credentials.json")).unwrap()
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Two managed accounts; b is the active default login.
fn two_accounts() -> Sandbox {
    let s = Sandbox::new();
    s.default_login("a@example.com", "at-a");
    assert!(s.haryx(&["add"], &[]).status.success());
    s.default_login("b@example.com", "at-b");
    assert!(s.haryx(&["add"], &[]).status.success());
    s
}

#[test]
fn relogin_of_inactive_account_stores_fresh_login_and_keeps_active_login() {
    let s = two_accounts();
    let out = s.haryx(
        &["login", "1"],
        &[("FAKE_EMAIL", "a@example.com"), ("FAKE_TOKEN", "at-a2")],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("Signed in Account 1: a@example.com"),
        "{}",
        text(&out)
    );
    assert!(
        s.claude_calls().contains("--email a@example.com"),
        "{}",
        s.claude_calls()
    );
    assert!(
        !s.stored_files_with("at-a2").is_empty(),
        "fresh login must be stored"
    );
    assert!(
        s.default_credentials().contains("at-b"),
        "active login untouched"
    );
    assert!(
        !s.store().join("login-staging").exists()
            || std::fs::read_dir(s.store().join("login-staging"))
                .unwrap()
                .next()
                .is_none(),
        "staging profile removed"
    );
}

#[test]
fn relogin_as_another_account_changes_nothing() {
    let s = two_accounts();
    let out = s.haryx(
        &["login", "1"],
        &[("FAKE_EMAIL", "c@example.com"), ("FAKE_TOKEN", "at-c")],
    );
    assert!(!out.status.success());
    assert!(
        text(&out).contains("You signed in as c@example.com"),
        "{}",
        text(&out)
    );
    assert!(s.stored_files_with("at-c").is_empty(), "nothing stored");
    assert_eq!(s.roster()["accounts"].as_object().unwrap().len(), 2);
}

#[test]
fn relogin_of_active_account_signs_in_on_the_default_profile() {
    let s = two_accounts();
    let out = s.haryx(
        &["login", "b@example.com"],
        &[("FAKE_EMAIL", "b@example.com"), ("FAKE_TOKEN", "at-b2")],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(s.default_credentials().contains("at-b2"));
    assert!(!s.stored_files_with("at-b2").is_empty());
}

#[test]
fn login_without_account_adds_new_slot_without_switching() {
    let s = two_accounts();
    let out = s.haryx(
        &["login", "--alias", "work"],
        &[("FAKE_EMAIL", "c@example.com"), ("FAKE_TOKEN", "at-c")],
    );
    assert!(out.status.success(), "{}", text(&out));
    let roster = s.roster();
    assert_eq!(roster["accounts"]["3"]["email"], "c@example.com");
    assert_eq!(roster["accounts"]["3"]["alias"], "work");
    assert_eq!(roster["activeAccountNumber"], 2);
    assert!(s.default_credentials().contains("at-b"));
    assert!(
        !s.claude_calls().contains("--email"),
        "{}",
        s.claude_calls()
    );
}

#[test]
fn failed_sign_in_changes_nothing() {
    let s = two_accounts();
    let out = s.haryx(
        &["login", "1"],
        &[
            ("FAKE_EMAIL", "a@example.com"),
            ("FAKE_TOKEN", "at-a2"),
            ("FAKE_FAIL", "1"),
        ],
    );
    assert!(!out.status.success());
    assert!(
        text(&out).contains("Sign-in did not finish"),
        "{}",
        text(&out)
    );
    assert!(s.stored_files_with("at-a2").is_empty());
}

#[test]
fn slot_and_alias_are_refused_with_an_account() {
    let s = two_accounts();
    let out = s.haryx(&["login", "1", "--slot", "4"], &[]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("only for a new account"),
        "{}",
        text(&out)
    );
}
