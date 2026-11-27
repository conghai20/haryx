//! `haryx login`, `logout`, `whoami` end to end: the real binary against a fake Keycloak
//! (`tiny_http`), a fake browser (`$BROWSER`) that follows the sign-in straight back, and on macOS a
//! fake `security` that keeps items as files.

// The fakes are `#!/bin/sh` scripts.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tempfile::TempDir;

/// Follows the sign-in URL back to its `redirect_uri` with `?$FAKE_QUERY`.
const FAKE_BROWSER: &str = r#"#!/bin/sh
echo "$1" >> "$HOME/browser.log"
redirect=$(printf '%s' "$1" | sed -e 's/.*redirect_uri=//' -e 's/&.*//' -e 's/%3A/:/g' -e 's#%2F#/#g')
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

/// The access token's payload: `{"sub":"u-test","email":"you@example.com","sid":"sid-test",
/// "identity_provider":"github","azp":"haryx-cli"}`.
const ACCESS_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1LXRlc3QiLCJlbWFpbCI6In\
    lvdUBleGFtcGxlLmNvbSIsInNpZCI6InNpZC10ZXN0IiwiaWRlbnRpdHlfcHJvdmlkZXIiOiJnaXRodWIiLCJhenAiOiJoYX\
    J5eC1jbGkifQ.c2ln";

const REALM: &str = "/realms/t";
const OIDC: &str = "/realms/t/protocol/openid-connect";

/// A fake Keycloak: discovery, token, revocation and device endpoints; records every request.
struct FakeKeycloak {
    issuer: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    server: Arc<tiny_http::Server>,
}

impl FakeKeycloak {
    fn start(token_status: u16) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let issuer = format!("{base}{REALM}");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (srv, reqs, iss) = (Arc::clone(&server), Arc::clone(&requests), issuer.clone());
        std::thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let url = req.url().to_owned();
                reqs.lock().unwrap().push((url.clone(), body));
                let (status, reply) = answer(&iss, &url, token_status);
                let _ =
                    req.respond(tiny_http::Response::from_string(reply).with_status_code(status));
            }
        });
        FakeKeycloak {
            issuer,
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

    /// The body of the first request to `path`.
    fn body(&self, path: &str) -> String {
        let requests = self.requests.lock().unwrap();
        requests.iter().find(|(p, _)| p == path).unwrap().1.clone()
    }
}

fn answer(issuer: &str, url: &str, token_status: u16) -> (u16, String) {
    match url.strip_prefix(REALM).unwrap_or_default() {
        "/.well-known/openid-configuration" => {
            let oidc = format!("{issuer}/protocol/openid-connect");
            let doc = json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{oidc}/auth"),
                "token_endpoint": format!("{oidc}/token"),
                "revocation_endpoint": format!("{oidc}/revoke"),
                "device_authorization_endpoint": format!("{oidc}/auth/device"),
            });
            (200, doc.to_string())
        }
        "/protocol/openid-connect/token" if token_status == 200 => {
            let tokens = json!({
                "access_token": ACCESS_TOKEN, "expires_in": 300, "refresh_token": "rt-test",
                "id_token": "id_never_kept", "token_type": "Bearer",
            });
            (200, tokens.to_string())
        }
        "/protocol/openid-connect/token" => (
            token_status,
            r#"{"error":"invalid_grant","error_description":"Code not valid"}"#.into(),
        ),
        "/protocol/openid-connect/auth/device" => {
            let device = json!({
                "device_code": "dc-test", "user_code": "WXYZ-ABCD",
                "verification_uri": format!("{issuer}/device"),
                "verification_uri_complete": format!("{issuer}/device?user_code=WXYZ-ABCD"),
                "expires_in": 600, "interval": 1,
            });
            (200, device.to_string())
        }
        _ => (200, String::new()),
    }
}

impl Drop for FakeKeycloak {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

struct Sandbox {
    dir: TempDir,
    keycloak: FakeKeycloak,
}

impl Sandbox {
    fn new(token_status: u16) -> Self {
        let sandbox = Sandbox {
            dir: tempfile::tempdir().unwrap(),
            keycloak: FakeKeycloak::start(token_status),
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
            .env("HARYX_SSO_ISSUER", &self.keycloak.issuer)
            .env("HARYX_API_URL", "http://127.0.0.1:9/api/v1")
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

/// `whoami.json` without `consents`, which the CLI starts emitting in plan task 11.
fn whoami_contract() -> Value {
    let mut v = contract("whoami.json");
    v.as_object_mut().unwrap().remove("consents");
    v
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
    assert!(same_shape(status, &whoami_contract()));

    let text = s.haryx(&["whoami"], "");
    assert!(String::from_utf8_lossy(&text.stdout).contains("Not signed in to HaryX"));
}

#[test]
fn login_through_the_browser_keeps_the_session_and_whoami_shows_it() {
    let s = Sandbox::new(200);
    let out = s.haryx(
        &["login", "--provider", "github", "--json"],
        "code=the-code",
    );
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
        "{}/protocol/openid-connect/auth?client_id=haryx-cli&response_type=code",
        s.keycloak.issuer
    )));
    assert!(url.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
    assert!(url.contains("&kc_idp_hint=github"));
    let token = format!("{OIDC}/token");
    assert_eq!(
        s.keycloak.paths(),
        [
            format!("{REALM}/.well-known/openid-configuration"),
            token.clone()
        ]
    );
    let exchange = s.keycloak.body(&token);
    assert!(exchange.starts_with("grant_type=authorization_code&code=the-code&"));
    assert!(exchange.contains("&code_verifier=") && exchange.ends_with("&client_id=haryx-cli"));

    let stored = std::fs::read_to_string(s.stored_session()).unwrap();
    assert!(!stored.contains("id_never_kept"));
    assert!(stored.contains(r#""sid":"sid-test""#));

    let whoami = json_lines(&s.haryx(&["whoami", "--json"], ""));
    assert_eq!(whoami[0]["signedIn"], true);
    assert_eq!(whoami[0]["user"]["provider"], "github");
    assert_eq!(whoami[0]["user"]["id"], "u-test");
    assert!(same_shape(&whoami[0], &whoami_contract()));
    let text = s.haryx(&["whoami"], "");
    assert!(
        String::from_utf8_lossy(&text.stdout)
            .contains("Signed in to HaryX as you@example.com (GitHub).")
    );
}

#[test]
fn login_without_a_provider_offers_every_method() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "--json"], "code=c");
    assert!(out.status.success());
    let url = json_lines(&out)[0]["url"].as_str().unwrap().to_owned();
    assert!(!url.contains("kc_idp_hint"), "{url}");
    assert!(url.contains("scope=openid%20offline_access"));
}

#[test]
fn no_browser_signs_in_with_the_device_grant() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "--no-browser", "--json"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = json_lines(&out);
    assert_eq!(events[0]["event"], "browser");
    assert_eq!(events[0]["opened"], false);
    assert_eq!(
        events[0]["url"],
        format!("{}/device?user_code=WXYZ-ABCD", s.keycloak.issuer)
    );
    assert!(same_shape(&events[0], &contract("login-events.json")[0]));
    assert_eq!(events[1]["event"], "signed_in");
    assert!(!s.home().join("browser.log").exists(), "no browser started");

    let device = s.keycloak.body(&format!("{OIDC}/auth/device"));
    assert!(device.contains("&code_challenge=") && device.contains("code_challenge_method=S256"));
    let poll = s.keycloak.body(&format!("{OIDC}/token"));
    assert!(poll.starts_with(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=dc-test&"
    ));
    assert!(poll.contains("&code_verifier="));

    let text = s.haryx(&["logout"], "");
    assert!(text.status.success());
    let out = s.haryx(&["login", "--no-browser"], "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("/device?user_code=WXYZ-ABCD")
            && stdout.contains("shows the code WXYZ-ABCD"),
        "{stdout}"
    );
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
    assert!(
        !s.keycloak.paths().iter().any(|p| p.ends_with("/token")),
        "no code, no exchange"
    );
    assert!(!s.stored_session().exists());
}

#[test]
fn a_refused_exchange_is_reported_in_text() {
    let s = Sandbox::new(400);
    let out = s.haryx(&["login"], "code=stale");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("The sign-in server refused: Code not valid"),
        "{stderr}"
    );
    assert!(!s.stored_session().exists());
}

#[test]
fn logout_revokes_the_refresh_token_and_forgets_the_session() {
    let s = Sandbox::new(200);
    assert!(s.haryx(&["login"], "code=c").status.success());
    let out = s.haryx(&["logout", "--json"], "");
    assert!(out.status.success());
    assert_eq!(json_lines(&out)[0]["signedIn"], false);
    assert_eq!(
        s.keycloak.body(&format!("{OIDC}/revoke")),
        "token=rt-test&token_type_hint=refresh_token&client_id=haryx-cli"
    );
    assert!(!s.stored_session().exists());

    let again = s.haryx(&["logout"], "");
    assert!(String::from_utf8_lossy(&again.stdout).contains("was not signed in"));
}

#[test]
fn a_supabase_session_is_signed_out_and_says_to_sign_in_again() {
    let s = Sandbox::new(200);
    let path = s.stored_session();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"access_token":"at","refresh_token":"rt-old","expires_at":4000000000,
            "user":{"id":"u","email":"you@example.com","provider":"github"},
            "project":"https://ygvcznwlugaxqqmlsssh.supabase.co"}"#,
    )
    .unwrap();

    let text = s.haryx(&["whoami"], "");
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains(
            "Not signed in to HaryX: HaryX now signs in with a WeCoAp ID; run 'haryx login'"
        ),
        "{stdout}"
    );
    let status = &json_lines(&s.haryx(&["whoami", "--json"], ""))[0];
    assert_eq!(status["signedIn"], false);
    assert!(same_shape(status, &whoami_contract()));

    assert!(s.haryx(&["logout"], "").status.success());
    assert!(!path.exists());
    assert!(s.keycloak.paths().is_empty(), "its tokens go nowhere");
}

#[test]
fn account_delete_points_at_the_wecoap_account_page() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["account", "delete", "--yes", "--json"], "");
    assert_eq!(out.status.code(), Some(1));
    let answer = &json_lines(&out)[0];
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["code"], "unsupported");
    let message = answer["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("{}/account", s.keycloak.issuer)),
        "{message}"
    );
}

#[test]
fn claude_login_is_untouched_by_the_new_top_level_login() {
    let s = Sandbox::new(200);
    let out = s.haryx(&["login", "2"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("use 'haryx ai login 2'"));
}
