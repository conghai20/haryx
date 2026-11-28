//! The fakes the HaryX account tests share: a fake Keycloak and a fake HaryX API (`tiny_http`), a
//! fake browser (`$BROWSER`) that follows the sign-in straight back, and on macOS a fake `security`
//! that keeps items as files. Each test binary uses part of it.
#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tempfile::TempDir;

/// Follows the sign-in URL back to its `redirect_uri` with `?$FAKE_QUERY`.
pub const FAKE_BROWSER: &str = r#"#!/bin/sh
echo "$1" >> "$HOME/browser.log"
redirect=$(printf '%s' "$1" | sed -e 's/.*redirect_uri=//' -e 's/&.*//' -e 's/%3A/:/g' -e 's#%2F#/#g')
curl -s "$redirect?$FAKE_QUERY" > /dev/null &
"#;

/// Keeps `-a/-s` items as files next to itself, as `security` would keep them in the Keychain.
pub const FAKE_SECURITY: &str = r#"#!/bin/sh
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
pub const ACCESS_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1LXRlc3QiLCJlbWFpbCI6In\
    lvdUBleGFtcGxlLmNvbSIsInNpZCI6InNpZC10ZXN0IiwiaWRlbnRpdHlfcHJvdmlkZXIiOiJnaXRodWIiLCJhenAiOiJoYX\
    J5eC1jbGkifQ.c2ln";

pub const REALM: &str = "/realms/t";
pub const OIDC: &str = "/realms/t/protocol/openid-connect";

/// A fake Keycloak: discovery, token, revocation and device endpoints; records every request.
pub struct FakeKeycloak {
    pub issuer: String,
    pub requests: Arc<Mutex<Vec<(String, String)>>>,
    pub server: Arc<tiny_http::Server>,
    /// What the token endpoint answers from now on: 200 or a refusal (`invalid_grant`).
    pub token_status: Arc<AtomicU16>,
}

impl FakeKeycloak {
    pub fn start(token_status: u16) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let issuer = format!("{base}{REALM}");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let token_status = Arc::new(AtomicU16::new(token_status));
        let status = Arc::clone(&token_status);
        let (srv, reqs, iss) = (Arc::clone(&server), Arc::clone(&requests), issuer.clone());
        std::thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let url = req.url().to_owned();
                reqs.lock().unwrap().push((url.clone(), body));
                let (status, reply) = answer(&iss, &url, status.load(Ordering::SeqCst));
                let _ =
                    req.respond(tiny_http::Response::from_string(reply).with_status_code(status));
            }
        });
        FakeKeycloak {
            issuer,
            requests,
            server,
            token_status,
        }
    }

    pub fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// The body of the first request to `path`.
    pub fn body(&self, path: &str) -> String {
        let requests = self.requests.lock().unwrap();
        requests.iter().find(|(p, _)| p == path).unwrap().1.clone()
    }
}

pub fn answer(issuer: &str, url: &str, token_status: u16) -> (u16, String) {
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

pub struct Sandbox {
    pub dir: TempDir,
    pub keycloak: FakeKeycloak,
    pub api: Option<FakeApi>,
}

impl Sandbox {
    pub fn new(token_status: u16) -> Self {
        let sandbox = Sandbox {
            dir: tempfile::tempdir().unwrap(),
            keycloak: FakeKeycloak::start(token_status),
            api: None,
        };
        write_exec(&sandbox.home().join("browser"), FAKE_BROWSER);
        write_exec(&sandbox.home().join("security"), FAKE_SECURITY);
        sandbox
    }

    /// With a fake HaryX API, and signed in through the browser.
    pub fn signed_in() -> Self {
        let mut sandbox = Sandbox::new(200);
        sandbox.api = Some(FakeApi::start());
        let out = sandbox.haryx(&["login"], "code=c");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        sandbox
    }

    pub fn api(&self) -> &FakeApi {
        self.api.as_ref().expect("a sandbox with an API")
    }

    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    pub fn command(&self, argv: &[&str], query: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_haryx"));
        command
            .args(argv)
            .current_dir(self.home())
            .env("HOME", self.home())
            .env("HARYX_HOME", self.home().join("store"))
            .env("HARYX_SECURITY_BIN", self.home().join("security"))
            .env("HARYX_SSO_ISSUER", &self.keycloak.issuer)
            .env(
                "HARYX_API_URL",
                self.api.as_ref().map_or(CLOSED_API, |a| &a.base),
            )
            .env("BROWSER", self.home().join("browser"))
            .env("FAKE_QUERY", query)
            .env("NO_COLOR", "1")
            .env_remove("XDG_DATA_HOME");
        command
    }

    pub fn haryx(&self, argv: &[&str], query: &str) -> Output {
        self.command(argv, query).output().unwrap()
    }

    /// Where the session ended up: the fake Keychain item on macOS, the sealed file elsewhere.
    pub fn stored_session(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.home().join("keychain/haryx-account.session")
        } else {
            self.home().join("store/account/session")
        }
    }
}

pub fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn json_lines(out: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line:?}")))
        .collect()
}

pub fn contract(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/contracts/haryx")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `whoami.json` as it is when signed out, or when the consents could not be read: no `consents`.
pub fn whoami_without_consents() -> Value {
    let mut v = contract("whoami.json");
    v.as_object_mut().unwrap().remove("consents");
    v
}

/// Same keys, recursively; values may differ.
pub fn same_shape(actual: &Value, expected: &Value) -> bool {
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

/// Nothing listens here.
const CLOSED_API: &str = "http://127.0.0.1:9/api/v1";

/// The machine the API knows besides this one.
pub const OTHER_MACHINE: &str = "c41d7e08-2b9a-4e35-a6f1-70d8e3b94a52";

/// What the fake API holds, and how it answers.
#[derive(Default)]
pub struct ApiState {
    pub sync: bool,
    pub machines: bool,
    /// Every token is refused (401), as after a sign-out from the web.
    pub revoked: bool,
    pub machine_ids: Vec<String>,
}

/// A fake HaryX API: consents, machines, export, and sync rows behind the Sync consent. Records
/// every request as (method, url, body).
pub struct FakeApi {
    pub base: String,
    pub state: Arc<Mutex<ApiState>>,
    requests: Arc<Mutex<Vec<(String, String, String)>>>,
    server: Arc<tiny_http::Server>,
}

impl FakeApi {
    pub fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}/api/v1", server.server_addr().to_ip().unwrap());
        let state = Arc::new(Mutex::new(ApiState {
            machine_ids: vec![OTHER_MACHINE.into()],
            ..ApiState::default()
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (srv, st, reqs) = (
            Arc::clone(&server),
            Arc::clone(&state),
            Arc::clone(&requests),
        );
        std::thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let (method, url) = (req.method().as_str().to_owned(), req.url().to_owned());
                let (status, reply) = api_answer(&mut st.lock().unwrap(), &method, &url, &body);
                reqs.lock().unwrap().push((method, url, body));
                let _ =
                    req.respond(tiny_http::Response::from_string(reply).with_status_code(status));
            }
        });
        FakeApi {
            base,
            state,
            requests,
            server,
        }
    }

    pub fn set(&self, change: impl FnOnce(&mut ApiState)) {
        change(&mut self.state.lock().unwrap());
    }

    pub fn requests(&self) -> Vec<(String, String, String)> {
        self.requests.lock().unwrap().clone()
    }

    /// The requests with this method and URL (under `/api/v1`).
    pub fn sent(&self, method: &str, url: &str) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|(m, u, _)| m == method && u == &format!("/api/v1{url}"))
            .map(|(_, _, body)| body)
            .collect()
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

fn api_error(status: u16, code: &str) -> (u16, String) {
    (status, json!({"error": code}).to_string())
}

fn consents_body(st: &ApiState) -> String {
    let consent = |purpose: &str, on: bool| {
        json!({"purpose": purpose, "granted": on,
               "version": on.then_some("2026-11-01"), "at": on.then_some("2026-11-20T09:14:00Z"),
               "history": []})
    };
    json!({"consents": [consent("sync", st.sync), consent("machines", st.machines)]}).to_string()
}

pub fn machine_json(id: &str, name: &str) -> Value {
    json!({"machineId": id, "name": name, "os": "linux", "version": "1.0.0",
           "lastSeenAt": "2026-11-25T10:00:00Z", "isThisMachine": false, "signedIn": true,
           "sessionId": "sid-other"})
}

fn api_answer(st: &mut ApiState, method: &str, url: &str, body: &str) -> (u16, String) {
    if st.revoked {
        return api_error(401, "unauthorized");
    }
    let url = url.strip_prefix("/api/v1").unwrap_or_default();
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    match (method, segments.as_slice()) {
        ("GET", ["consents"]) => (200, consents_body(st)),
        ("PUT" | "DELETE", ["consents", purpose]) => {
            let on = method == "PUT";
            match *purpose {
                "sync" => st.sync = on,
                "machines" => st.machines = on,
                _ => return api_error(400, "invalid_purpose"),
            }
            if !on && !query.starts_with("source=") {
                return api_error(400, "invalid_source");
            }
            (200, consents_body(st))
        }
        ("PUT", ["machines", "self"]) if !st.machines => api_error(403, "consent_required"),
        ("PUT", ["machines", "self"]) => {
            let sent: Value = serde_json::from_str(body).unwrap();
            let id = sent["machineId"].as_str().unwrap().to_owned();
            if !st.machine_ids.contains(&id) {
                st.machine_ids.push(id.clone());
            }
            (
                200,
                machine_json(&id, sent["name"].as_str().unwrap()).to_string(),
            )
        }
        ("GET", ["machines"]) => {
            let list: Vec<Value> = st
                .machine_ids
                .iter()
                .map(|id| machine_json(id, "Box"))
                .collect();
            (200, json!({"machines": list}).to_string())
        }
        ("PATCH", ["machines", id]) if st.machine_ids.iter().any(|m| m == id) => {
            let sent: Value = serde_json::from_str(body).unwrap();
            (
                200,
                machine_json(id, sent["name"].as_str().unwrap()).to_string(),
            )
        }
        ("PATCH", ["machines", _]) => api_error(404, "not_found"),
        ("GET", ["me", "export"]) => (200, r#"{"exportedAt":"2026-11-27T00:00:00Z"}"#.into()),
        (_, ["sync", ..]) if !st.sync => api_error(403, "consent_required"),
        ("GET", ["sync", "rows"]) => (200, "[]".into()),
        _ => api_error(404, "not_found"),
    }
}
