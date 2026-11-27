//! `haryx login`, `logout`, `whoami` end to end: the real binary against a fake Supabase
//! (`tiny_http`), a fake browser (`$BROWSER`) that follows the sign-in straight back, and on macOS a
//! fake `security` that keeps items as files.

// The fakes are `#!/bin/sh` scripts.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tempfile::TempDir;

/// Follows the sign-in URL back to its `redirect_to` with `?$FAKE_QUERY`.
const FAKE_BROWSER: &str = r#"#!/bin/sh
echo "$1" >> "$HOME/browser.log"
redirect=$(printf '%s' "$1" | sed -e 's/.*redirect_to=//' -e 's/&.*//' -e 's/%3A/:/g' -e 's#%2F#/#g')
curl -s "$redirect?$FAKE_QUERY" > /dev/null &
"#;

/// Keeps `-a/-s` items as files next to itself, as `security` would keep them in the Keychain.
const FAKE_SECURITY: &str = r#"#!/bin/sh
dir="$(dirname "$0")/keychain"; mkdir -p "$dir"
if [ "$1" = "-i" ]; then set -- $(cat | tr -d '"'); fi
op="$1"; shift
while [ $# -gt 0 ]; do
  case "$1" in -a) acct="$2"; shift 2;; -s) svc="$2"; shift 2;; -X) hex="$2"; shift 2;; *) shift;; esac
done
f="$dir/$svc.$acct"
case "$op" in
  add-generic-password) printf '%s' "$hex" | xxd -r -p > "$f";;
  find-generic-password) [ -f "$f" ] || exit 44; cat "$f"; echo;;
  delete-generic-password) [ -f "$f" ] || exit 44; rm "$f";;
  *) exit 1;;
esac
"#;

const SESSION: &str = r#"{"access_token":"at-test","refresh_token":"rt-test","expires_in":3600,
  "expires_at":4000000000,"provider_token":"gho_never_kept",
  "user":{"id":"u-test","email":"you@example.com","app_metadata":{"provider":"github"}}}"#;

/// A fake Supabase Auth: answers the token and logout endpoints, records every request.
struct FakeSupabase {
    base: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    server: Arc<tiny_http::Server>,
}

impl FakeSupabase {
    fn start(token_status: u16) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (srv, reqs) = (Arc::clone(&server), Arc::clone(&requests));
        std::thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let url = req.url().to_owned();
                reqs.lock().unwrap().push((url.clone(), body));
                let (status, reply) = if url.starts_with("/auth/v1/token") {
                    let reply = if token_status == 200 {
                        SESSION
                    } else {
                        r#"{"msg":"invalid flow state"}"#
                    };
                    (token_status, reply)
                } else {
                    (204, "")
                };
                let _ =
                    req.respond(tiny_http::Response::from_string(reply).with_status_code(status));
            }
        });
        FakeSupabase {
            base,
            requests,
            server,
        }
    }

    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }
}

impl Drop for FakeSupabase {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

struct Sandbox {
    dir: TempDir,
    supabase: FakeSupabase,
}

impl Sandbox {
    fn new(token_status: u16) -> Self {
        let sandbox = Sandbox {
            dir: tempfile::tempdir().unwrap(),
            supabase: FakeSupabase::start(token_status),
        };
        write_exec(&sandbox.home().join("browser"), FAKE_BROWSER);
        write_exec(&sandbox.home().join("security"), FAKE_SECURITY);
        sandbox
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn command(&self, argv: &[&str], query: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_haryx"));
        command
            .args(argv)
            .current_dir(self.home())
            .env("HOME", self.home())
            .env("HARYX_HOME", self.home().join("store"))
            .env("HARYX_SECURITY_BIN", self.home().join("security"))
            .env("HARYX_SUPABASE_URL", &self.supabase.base)
            .env("HARYX_SUPABASE_KEY", "pk-test")
            .env("BROWSER", self.home().join("browser"))
            .env("FAKE_QUERY", query)
            .env("NO_COLOR", "1")
            .env_remove("XDG_DATA_HOME");
        command
    }

    fn haryx(&self, argv: &[&str], query: &str) -> Output {
        self.command(argv, query).output().unwrap()
    }

    /// Where the session ended up: the fake Keychain item on macOS, the sealed file elsewhere.
    fn stored_session(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.home().join("keychain/haryx-account.session")
        } else {
            self.home().join("store/account/session")
        }
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn json_lines(out: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line:?}")))
        .collect()
}

fn contract(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/contracts/haryx")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Same keys, recursively; values may differ.
fn same_shape(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            a.len() == e.len()
                && e.iter()
                    .all(|(k, ev)| a.get(k).is_some_and(|av| same_shape(av, ev)))
        }
        (Value::Null, _) | (_, Value::Null) => true,
        (a, e) => std::mem::discriminant(a) == std::mem::discriminant(e),
    }
}

#[test]
fn whoami_signed_out_matches_the_contract() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["whoami", "--json"], "");
    assert!(out.status.success());
    let status = &json_lines(&out)[0];
    assert_eq!(status["signedIn"], false);
    assert_eq!(status["user"], Value::Null);
    assert!(same_shape(status, &contract("whoami.json")));

    let text = s.haryx(&["whoami"], "");
    assert!(String::from_utf8_lossy(&text.stdout).contains("Not signed in to HaryX"));
}

#[test]
fn login_through_the_browser_keeps_the_session_and_whoami_shows_it() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "--json"], "code=the-code");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = json_lines(&out);
    let fixture = contract("login-events.json");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "browser");
    assert_eq!(events[0]["opened"], true);
    assert!(same_shape(&events[0], &fixture[0]));
    assert_eq!(events[1]["event"], "signed_in");
    assert_eq!(events[1]["user"]["email"], "you@example.com");
    assert!(same_shape(&events[1], &fixture[1]));

    let url = events[0]["url"].as_str().unwrap();
    assert!(url.starts_with(&format!(
        "{}/auth/v1/authorize?provider=github&redirect_to=http%3A%2F%2F127.0.0.1%3A",
        s.supabase.base
    )));
    assert_eq!(
        s.supabase.paths(),
        ["/auth/v1/token?grant_type=pkce".to_owned()]
    );
    let exchange = &s.supabase.requests.lock().unwrap()[0].1;
    assert!(exchange.contains(r#""auth_code":"the-code""#));

    let stored = std::fs::read_to_string(s.stored_session()).unwrap();
    assert!(!stored.contains("gho_never_kept"));

    let whoami = json_lines(&s.haryx(&["whoami", "--json"], ""));
    assert_eq!(whoami[0]["signedIn"], true);
    assert_eq!(whoami[0]["user"]["provider"], "github");
    assert!(same_shape(&whoami[0], &contract("whoami.json")));
    let text = s.haryx(&["whoami"], "");
    assert!(
        String::from_utf8_lossy(&text.stdout)
            .contains("Signed in to HaryX as you@example.com (GitHub).")
    );
}

#[test]
fn no_browser_prints_the_url_and_still_completes() {
    let s = Sandbox::new(200);
    let mut child: Child = s
        .command(&["login", "--no-browser", "--json"], "")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    let event: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(event["opened"], false);
    assert!(!s.home().join("browser.log").exists(), "no browser started");

    // Open the link "by hand": go straight to the redirect it names.
    let url = event["url"].as_str().unwrap();
    let redirect = url
        .split("redirect_to=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .replace("%3A", ":")
        .replace("%2F", "/");
    let rest_of_url = redirect.strip_prefix("http://").unwrap();
    let (host, path) = rest_of_url.split_once('/').unwrap();
    assert!(path.starts_with("callback/") && path.len() > "callback/".len() + 16);
    let mut stream = std::net::TcpStream::connect(host).unwrap();
    write!(
        stream,
        "GET /{path}?code=manual HTTP/1.1\r\nHost: {host}\r\n\r\n"
    )
    .unwrap();
    let mut page = String::new();
    stream.read_to_string(&mut page).unwrap();
    assert!(page.contains("<title>Signed in to HaryX</title>"));
    assert!(
        page.contains("you@example.com"),
        "the page names who signed in"
    );

    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert!(child.wait().unwrap().success());
    assert!(rest.contains(r#""event":"signed_in""#), "{rest}");
}

#[test]
fn a_denied_sign_in_is_an_error_event_and_stores_nothing() {
    let s = Sandbox::new(200);
    let out = s.haryx(
        &["login", "--json"],
        "error=access_denied&error_description=The+user+denied+access",
    );
    assert_eq!(out.status.code(), Some(1));
    let events = json_lines(&out);
    assert_eq!(events[1]["event"], "error");
    assert_eq!(events[1]["code"], "denied");
    assert!(same_shape(&events[1], &contract("login-events.json")[2]));
    assert!(s.supabase.paths().is_empty(), "no code, no exchange");
    assert!(!s.stored_session().exists());
}

#[test]
fn a_refused_exchange_is_reported_in_text() {
    let s = Sandbox::new(400);
    let out = s.haryx(&["login"], "code=stale");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("The sign-in server refused: invalid flow state"),
        "{stderr}"
    );
    assert!(!s.stored_session().exists());
}

#[test]
fn logout_tells_the_server_and_forgets_the_session() {
    let s = Sandbox::new(200);
    assert!(s.haryx(&["login"], "code=c").status.success());
    let out = s.haryx(&["logout", "--json"], "");
    assert!(out.status.success());
    assert_eq!(json_lines(&out)[0]["signedIn"], false);
    assert!(
        s.supabase
            .paths()
            .contains(&"/auth/v1/logout?scope=local".to_owned())
    );
    assert!(!s.stored_session().exists());

    let again = s.haryx(&["logout"], "");
    assert!(String::from_utf8_lossy(&again.stdout).contains("was not signed in"));
}

#[test]
fn claude_login_is_untouched_by_the_new_top_level_login() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "2"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("use 'haryx ai login 2'"));
}
