//! The HaryX API's account endpoints, as the signed-in user: consents, the machine registry and
//! the data export (`server/api/internal/{consent,machines,export}`). Every call goes through
//! [`Api`], so a refused token is renewed once and only Keycloak's `invalid_grant` signs out, as
//! for sync.

use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::contracts::{Consents, Machine, Machines, Purpose};
use crate::machine;
use crate::oauth::http::HttpResponse;
use crate::sync::SyncError;
use crate::sync::remote::{Api, Method, encode};

/// The privacy policy version a grant records. It must match the version of the published HaryX
/// privacy policy that the consent text links to: bump the two together.
pub const POLICY_VERSION: &str = "2026-11-01";

/// How long one account call may take.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// Where the user answered a consent question: the CLI itself, or an app that runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cli,
    Macos,
    Windows,
}

impl Source {
    pub fn id(self) -> &'static str {
        match self {
            Source::Cli => "cli",
            Source::Macos => "macos",
            Source::Windows => "windows",
        }
    }
}

/// `path` as the signed-in user; `payload` is sent as JSON unless it is null.
fn send(
    api: &Api,
    method: Method,
    path: &str,
    payload: &Value,
    timeout: Duration,
    expected: &[u16],
) -> Result<Option<HttpResponse>, SyncError> {
    if payload.is_null() {
        return api.call(method, path, &[], &[], timeout, expected);
    }
    let body = payload.to_string();
    let headers = [("content-type", "application/json")];
    api.call(method, path, &headers, body.as_bytes(), timeout, expected)
}

/// A call with no status but 2xx expected.
fn fetch(
    api: &Api,
    method: Method,
    path: &str,
    payload: &Value,
    timeout: Duration,
) -> Result<HttpResponse, SyncError> {
    send(api, method, path, payload, timeout, &[]).map(|resp| resp.expect("no expected status"))
}

fn decode<T: DeserializeOwned>(resp: &HttpResponse) -> Result<T, SyncError> {
    serde_json::from_slice(&resp.body)
        .map_err(|e| SyncError::Server(format!("unexpected response: {e}")))
}

/// The user's consents (`consents.json`), waiting at most `timeout`.
pub fn consents(api: &Api, timeout: Duration) -> Result<Consents, SyncError> {
    decode(&fetch(
        api,
        Method::Get,
        "/consents",
        &Value::Null,
        timeout,
    )?)
}

/// Grant `purpose` at [`POLICY_VERSION`]. Answers the consents as they now stand.
pub fn grant(api: &Api, purpose: Purpose, source: Source) -> Result<Consents, SyncError> {
    let path = format!("/consents/{}", purpose.id());
    let payload = json!({"version": POLICY_VERSION, "source": source.id()});
    decode(&fetch(api, Method::Put, &path, &payload, TIMEOUT)?)
}

/// Withdraw `purpose`: the server erases its data in the same transaction.
pub fn withdraw(api: &Api, purpose: Purpose, source: Source) -> Result<Consents, SyncError> {
    let path = format!("/consents/{}?source={}", purpose.id(), source.id());
    decode(&fetch(api, Method::Delete, &path, &Value::Null, TIMEOUT)?)
}

/// The API marks the machine signed in with the caller's session; this one is also marked by its
/// own id, which holds after a new sign-in until the next heartbeat.
fn mark(mut machine: Machine, this_machine: &str) -> Machine {
    machine.is_this_machine |= machine.machine_id == this_machine;
    machine
}

/// The user's machines (`machines.json`); `this_machine` is [`machine::machine_id`].
pub fn machines(api: &Api, this_machine: &str) -> Result<Machines, SyncError> {
    let list: Machines = decode(&fetch(
        api,
        Method::Get,
        "/machines",
        &Value::Null,
        TIMEOUT,
    )?)?;
    Ok(Machines {
        machines: list
            .machines
            .into_iter()
            .map(|m| mark(m, this_machine))
            .collect(),
    })
}

/// Rename machine `id`. The name stays: heartbeats never overwrite it.
pub fn rename_machine(
    api: &Api,
    id: &str,
    name: &str,
    this_machine: &str,
) -> Result<Machine, SyncError> {
    let path = format!("/machines/{}", encode(id));
    let payload = json!({"name": name});
    let Some(resp) = send(api, Method::Patch, &path, &payload, TIMEOUT, &[404])? else {
        return Err(SyncError::Invalid(format!(
            "No machine {id} on your HaryX ID. '{} account machines' lists them.",
            crate::PROG
        )));
    };
    Ok(mark(decode(&resp)?, this_machine))
}

/// Everything HaryX keeps about the user, as the API's JSON file.
pub fn export(api: &Api) -> Result<Vec<u8>, SyncError> {
    Ok(fetch(api, Method::Get, "/me/export", &Value::Null, TIMEOUT)?.body)
}

/// Tell the machine registry this machine is here: `Ok(false)` when the user has not granted the
/// machines consent, which is not a failure.
pub fn heartbeat(api: &Api, store_root: &Path) -> Result<bool, SyncError> {
    let payload = json!({
        "machineId": machine::machine_id(store_root),
        "name": machine::default_name(),
        "os": machine::os(),
        "version": crate::VERSION,
    });
    match fetch(api, Method::Put, "/machines/self", &payload, TIMEOUT) {
        Ok(_) => Ok(true),
        Err(SyncError::ConsentRequired) => Ok(false),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::client::fake::{self, REALM};
    use crate::account::{Backend, Session, SessionStore, User};
    use crate::oauth::tests::stub::{Recorded, Reply, Stub};

    const ID: &str = "9b2e5c1a-4f0d-4a67-8d3e-1c7a5b0e2f91";
    const OTHER: &str = "c41d7e08-2b9a-4e35-a6f1-70d8e3b94a52";

    struct World {
        _dir: tempfile::TempDir,
        api: Stub,
        _keycloak: Stub,
        client: Api,
    }

    fn world(route: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> World {
        let api = Stub::start(route);
        let keycloak = fake::keycloak(|_| Reply::json(500, "{}"));
        let backend = Backend::new(
            &format!("{}{REALM}", keycloak.base()),
            &format!("{}/api/v1", api.base()),
        );
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::file(dir.path().join("account"));
        store
            .save(&Session {
                access_token: "at".into(),
                refresh_token: "rt".into(),
                expires_at: jiff::Timestamp::now().as_second() + 3600,
                user: User {
                    id: "u".into(),
                    email: "you@example.com".into(),
                    provider: "github".into(),
                },
                issuer: backend.issuer().into(),
                sid: "sid".into(),
            })
            .unwrap();
        let client = Api::new(&backend, &store, "at");
        World {
            _dir: dir,
            api,
            _keycloak: keycloak,
            client,
        }
    }

    fn machine_json(id: &str, this: bool) -> Value {
        json!({"machineId": id, "name": "Mac", "os": "macos", "version": "1.0.0",
               "lastSeenAt": "2026-11-25T10:00:00Z", "isThisMachine": this, "signedIn": true,
               "sessionId": "s"})
    }

    #[test]
    fn a_grant_sends_the_policy_version_and_the_source() {
        let body = include_str!("../../../../../packages/contracts/haryx/consents.json");
        let w = world(move |_| Reply::json(200, body));
        let consents = grant(&w.client, Purpose::Machines, Source::Macos).unwrap();
        assert_eq!(consents.consents.len(), 2);
        let req = &w.api.requests()[0];
        assert_eq!(
            (req.method.as_str(), req.path.as_str()),
            ("PUT", "/api/v1/consents/machines")
        );
        let sent: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(sent, json!({"version": "2026-11-01", "source": "macos"}));

        withdraw(&w.client, Purpose::Sync, Source::Cli).unwrap();
        let req = &w.api.requests()[1];
        assert_eq!(
            (req.method.as_str(), req.path.as_str()),
            ("DELETE", "/api/v1/consents/sync?source=cli")
        );
    }

    #[test]
    fn this_machine_is_also_found_by_its_id() {
        let w = world(|req| match req.method.as_str() {
            "PATCH" => Reply::json(200, &machine_json(ID, false).to_string()),
            _ => Reply::json(
                200,
                &json!({"machines": [machine_json(ID, false), machine_json(OTHER, false)]})
                    .to_string(),
            ),
        });
        let list = machines(&w.client, ID).unwrap().machines;
        assert!(list[0].is_this_machine && !list[1].is_this_machine);
        let renamed = rename_machine(&w.client, ID, "Work", ID).unwrap();
        assert!(renamed.is_this_machine);
        let req = &w.api.requests()[1];
        assert_eq!(req.path, format!("/api/v1/machines/{ID}"));
        assert_eq!(req.body, r#"{"name":"Work"}"#);
    }

    #[test]
    fn renaming_a_machine_that_is_not_yours_says_so() {
        let w = world(|_| Reply::json(404, r#"{"error":"not_found"}"#));
        let err = rename_machine(&w.client, OTHER, "Work", ID).unwrap_err();
        assert!(matches!(&err, SyncError::Invalid(m) if m.contains("account machines")));
    }

    #[test]
    fn a_heartbeat_names_this_machine_and_without_consent_is_skipped() {
        let w = world(|_| Reply::json(200, &machine_json(ID, true).to_string()));
        let root = tempfile::tempdir().unwrap();
        assert!(heartbeat(&w.client, root.path()).unwrap());
        let sent: Value = serde_json::from_str(&w.api.requests()[0].body).unwrap();
        assert_eq!(sent["machineId"], machine::machine_id(root.path()));
        assert_eq!(sent["os"], machine::os());
        assert_eq!(sent["version"], crate::VERSION);
        assert!(!sent["name"].as_str().unwrap().is_empty());

        let off = world(|_| Reply::json(403, r#"{"error":"consent_required"}"#));
        assert!(!heartbeat(&off.client, root.path()).unwrap());
        let other = world(|_| Reply::json(403, r#"{"error":"forbidden_client"}"#));
        assert!(heartbeat(&other.client, root.path()).is_err());
    }
}
