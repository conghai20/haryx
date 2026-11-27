//! [`Api`] against a fake HaryX API and a fake Keycloak, both on `tiny_http`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::SyncError;
use super::profiles::Objects;
use super::remote::{Api, Remote};
use crate::account::client::fake::{self, REALM, TOKEN};
use crate::account::{AccountError, Backend, Session, SessionStore, User};
use crate::oauth::tests::stub::{Recorded, Reply, Stub, closed_base};

const DATE: &str = "Thu, 26 Nov 2026 16:00:00 GMT";
const DATE_UNIX: i64 = 1_795_708_800;
/// The token the fake API refuses as expired or revoked.
const STALE: &str = "at-stale";
const GOOD: &str = "at-good";

/// What the fake API keeps: rows by (kind, key) as (body, version), files by name.
#[derive(Default)]
struct State {
    rows: BTreeMap<(String, String), (String, i64)>,
    files: BTreeMap<String, String>,
}

fn row_json(kind: &str, key: &str, body: &str, version: i64) -> Value {
    json!({"kind": kind, "key": key, "body": body, "version": version,
           "updatedAt": format!("2026-11-25T10:00:{:02}Z", version)})
}

fn error(status: u16, code: &str) -> Reply {
    Reply::json(status, &json!({"error": code}).to_string())
}

/// The API's routes, as built in `server/api/internal/sync`.
fn serve(state: &Mutex<State>, req: &Recorded) -> Reply {
    if req.header("authorization") != Some(&format!("Bearer {GOOD}"))
        && !req.header("authorization").is_some_and(|h| h.contains('.'))
    {
        return error(401, "unauthorized");
    }
    let mut st = state.lock().unwrap();
    let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
    let path = path.strip_prefix("/api/v1").unwrap_or_default();
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    match (req.method.as_str(), segments.as_slice()) {
        ("GET", ["sync", "rows"]) => {
            let kind = query.strip_prefix("kind=");
            let list: Vec<Value> = st
                .rows
                .iter()
                .filter(|((k, _), _)| kind.is_none_or(|want| want == k))
                .map(|((k, key), (body, v))| row_json(k, key, body, *v))
                .collect();
            Reply::json(200, &Value::from(list).to_string())
        }
        ("POST", ["sync", "rows"]) => {
            let v: Value = serde_json::from_str(&req.body).unwrap();
            let text = |f: &str| v[f].as_str().unwrap().to_owned();
            let at = (text("kind"), text("key"));
            if st.rows.contains_key(&at) {
                return error(409, "exists");
            }
            st.rows.insert(at.clone(), (text("body"), 1));
            Reply::json(201, &row_json(&at.0, &at.1, &text("body"), 1).to_string())
        }
        ("PUT", ["sync", "rows", kind, key]) => {
            let Some(version) = req.header("if-match").and_then(|v| v.parse::<i64>().ok()) else {
                return error(428, "if_match_required");
            };
            let body = serde_json::from_str::<Value>(&req.body).unwrap()["body"]
                .as_str()
                .unwrap()
                .to_owned();
            match st.rows.get_mut(&((*kind).to_owned(), (*key).to_owned())) {
                None => error(404, "not_found"),
                Some((_, v)) if *v != version => error(412, "version_mismatch"),
                Some(row) => {
                    *row = (body.clone(), version + 1);
                    Reply::json(200, &row_json(kind, key, &body, version + 1).to_string())
                }
            }
        }
        ("DELETE", ["sync", "rows", kind, key]) => {
            st.rows.remove(&((*kind).to_owned(), (*key).to_owned()));
            Reply::json(204, "")
        }
        ("DELETE", ["sync", "rows"]) => {
            st.rows.clear();
            Reply::json(204, "")
        }
        ("GET", ["profiles"]) => {
            let list: Vec<Value> = st
                .files
                .iter()
                .map(|(name, body)| {
                    json!({"name": name, "bytes": body.len(), "updatedAt": "2026-11-25T10:00:00Z"})
                })
                .collect();
            Reply::json(200, &Value::from(list).to_string())
        }
        ("PUT", ["profiles", name]) => {
            st.files.insert((*name).to_owned(), req.body.clone());
            Reply::json(
                200,
                &json!({"name": name, "bytes": req.body.len(), "updatedAt": "2026-11-25T10:00:00Z"})
                    .to_string(),
            )
        }
        ("GET", ["profiles", name]) => match st.files.get(*name) {
            Some(body) => Reply::json(200, body),
            None => error(404, "not_found"),
        },
        ("DELETE", ["profiles", name]) => {
            st.files.remove(*name);
            Reply::json(204, "")
        }
        _ => error(404, "not_found"),
    }
}

struct World {
    _dir: tempfile::TempDir,
    store: SessionStore,
    keycloak: Stub,
    api: Stub,
    backend: Backend,
}

impl World {
    /// `refusing`: a reply that replaces every route (after the token check, when `None`).
    fn new(
        keycloak: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
        refusing: impl Fn(&Recorded) -> Option<Reply> + Send + Sync + 'static,
    ) -> World {
        let state = Arc::new(Mutex::new(State::default()));
        let api = Stub::start(move |req| {
            refusing(req)
                .unwrap_or_else(|| serve(&state, req))
                .header("Date", DATE)
        });
        let keycloak = fake::keycloak(keycloak);
        let backend = Backend::new(
            &format!("{}{REALM}/", keycloak.base()),
            &format!("{}/api/v1", api.base()),
        );
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::file(dir.path().join("account"));
        World {
            _dir: dir,
            store,
            keycloak,
            api,
            backend,
        }
    }

    fn plain() -> World {
        World::new(renewing, |_| None)
    }

    /// Signed in with `access`, good for another hour.
    fn signed_in(&self, access: &str) -> Api {
        self.store
            .save(&Session {
                access_token: access.into(),
                refresh_token: "rt-1".into(),
                expires_at: jiff::Timestamp::now().as_second() + 3600,
                user: User {
                    id: "u-1".into(),
                    email: "you@example.com".into(),
                    provider: "github".into(),
                },
                issuer: self.backend.issuer().into(),
                sid: "sid-1".into(),
            })
            .unwrap();
        Api::new(&self.backend, &self.store, access)
    }

    fn api_requests(&self) -> Vec<Recorded> {
        self.api.requests()
    }
}

fn renewing(_: &Recorded) -> Reply {
    Reply::json(200, &fake::tokens(None, "rt-2"))
}

#[test]
fn rows_round_trip_with_versions_and_the_server_clock() {
    let w = World::plain();
    let api = w.signed_in(GOOD);

    let row = api.insert("settings", "", "a").unwrap().unwrap();
    assert_eq!((row.version, row.body.as_str()), (1, "a"));
    assert_eq!(row.updated_at, 1_795_600_801);
    assert_eq!(api.insert("settings", "", "b").unwrap(), None, "409");
    api.insert("account", "ab12", "x").unwrap().unwrap();

    let moved = api.update("settings", "", "b", 1).unwrap().unwrap();
    assert_eq!((moved.version, moved.body.as_str()), (2, "b"));
    assert_eq!(api.update("settings", "", "c", 1).unwrap(), None, "412");
    assert_eq!(api.update("settings", "ff", "c", 1).unwrap(), None, "404");

    let all = Remote::list(&api).unwrap();
    assert_eq!(all.len(), 2);
    let listing = api.list_kind("settings").unwrap();
    assert_eq!(listing.rows, vec![moved]);
    assert_eq!(listing.server_now, Some(DATE_UNIX));

    Remote::delete(&api, "settings", "").unwrap();
    Remote::delete(&api, "settings", "").unwrap(); // already gone: still fine
    assert_eq!(Remote::list(&api).unwrap().len(), 1);
    api.delete_all().unwrap();
    assert!(Remote::list(&api).unwrap().is_empty());

    let reqs = w.api_requests();
    let put = reqs.iter().find(|r| r.method == "PUT").unwrap();
    assert_eq!(put.path, "/api/v1/sync/rows/settings/");
    assert_eq!(put.header("if-match"), Some("1"));
    assert_eq!(
        serde_json::from_str::<Value>(&put.body).unwrap(),
        json!({"body": "b"})
    );
    assert!(
        reqs.iter()
            .any(|r| r.path == "/api/v1/sync/rows?kind=settings")
    );
    assert!(
        reqs.iter()
            .all(|r| r.header("authorization") == Some("Bearer at-good"))
    );
    assert!(fake::at(&w.keycloak, TOKEN).is_empty(), "no renewal needed");
}

#[test]
fn a_401_renews_once_and_retries_with_the_new_token() {
    let w = World::plain();
    let api = w.signed_in(STALE);
    api.insert("settings", "", "a").unwrap().unwrap();
    assert_eq!(Remote::list(&api).unwrap().len(), 1);

    assert_eq!(fake::at(&w.keycloak, TOKEN).len(), 1, "renewed once");
    let reqs = w.api_requests();
    assert_eq!(reqs.len(), 3, "the refused call, its retry, then the list");
    assert_eq!(reqs[0].header("authorization"), Some("Bearer at-stale"));
    let renewed = reqs[1].header("authorization").unwrap();
    assert_ne!(renewed, "Bearer at-stale");
    assert_eq!(reqs[2].header("authorization"), Some(renewed));
    assert_eq!(reqs[0].body, reqs[1].body, "the same request again");
    assert_eq!(w.store.load().unwrap().unwrap().refresh_token, "rt-2");
}

#[test]
fn a_401_after_a_renewal_is_an_error_and_keeps_the_session() {
    let w = World::new(renewing, |_| Some(error(401, "unauthorized")));
    let api = w.signed_in(GOOD);
    let err = Remote::list(&api).unwrap_err();
    assert!(
        matches!(&err, SyncError::Server(m) if m.contains("401")),
        "{err:?}"
    );
    assert_eq!(err.code(), "server");
    assert_eq!(w.api_requests().len(), 2, "one retry, no more");
    assert_eq!(fake::at(&w.keycloak, TOKEN).len(), 1);
    assert!(w.store.load().unwrap().is_some(), "still signed in");
}

#[test]
fn a_refused_renewal_signs_out() {
    let w = World::new(
        |_| {
            Reply::json(
                400,
                r#"{"error":"invalid_grant","error_description":"Session not active"}"#,
            )
        },
        |_| None,
    );
    let api = w.signed_in(STALE);
    assert_eq!(
        Remote::list(&api).unwrap_err(),
        SyncError::Account(AccountError::SignedOut("Session not active".into()))
    );
    assert_eq!(w.store.load().unwrap(), None);
    assert_eq!(w.api_requests().len(), 1, "no retry without a new token");
}

#[test]
fn a_renewal_that_fails_otherwise_keeps_the_session() {
    let w = World::new(|_| Reply::json(503, "{}"), |_| None);
    let api = w.signed_in(STALE);
    assert!(matches!(
        Remote::list(&api).unwrap_err(),
        SyncError::Account(AccountError::Network(_))
    ));
    assert!(w.store.load().unwrap().is_some());
}

#[test]
fn no_consent_is_its_own_error() {
    let w = World::new(renewing, |_| Some(error(403, "consent_required")));
    let api = w.signed_in(GOOD);
    let err = api.insert("settings", "", "a").unwrap_err();
    assert_eq!(err, SyncError::ConsentRequired);
    assert_eq!(err.code(), "consent_required");
    assert_eq!(
        Objects::get(&api, "ab").unwrap_err(),
        SyncError::ConsentRequired
    );

    let w = World::new(renewing, |_| Some(error(403, "forbidden")));
    let err = Remote::list(&w.signed_in(GOOD)).unwrap_err();
    assert!(matches!(&err, SyncError::Server(m) if m.contains("403")));
}

#[test]
fn server_limits_say_what_to_do() {
    let w = World::new(renewing, |req| {
        Some(match req.method.as_str() {
            "POST" => error(422, "row_limit"),
            "PUT" => error(422, "profile_limit"),
            _ => error(413, "body_too_large"),
        })
    });
    let api = w.signed_in(GOOD);
    let err = api.insert("account", "ab", "x").unwrap_err();
    assert!(
        matches!(&err, SyncError::Invalid(m) if m.contains("500") && m.contains("sync forget")),
        "{err:?}"
    );
    let err = api.put("ab", b"file").unwrap_err();
    assert!(
        matches!(&err, SyncError::Invalid(m) if m.contains("10 profile files")),
        "{err:?}"
    );
    let err = Remote::delete(&api, "account", "ab").unwrap_err();
    assert!(matches!(&err, SyncError::Invalid(m) if m.contains("body_too_large")));
}

#[test]
fn profile_files_are_named_by_their_key_alone() {
    let w = World::plain();
    let api = w.signed_in(GOOD);
    let name = "0123456789abcdef";
    assert_eq!(Objects::get(&api, name).unwrap(), None);
    api.put(name, b"sealed bytes").unwrap();
    api.put("ab", b"other").unwrap();
    assert_eq!(
        Objects::get(&api, name).unwrap().as_deref(),
        Some(&b"sealed bytes"[..])
    );
    assert_eq!(
        Objects::list(&api).unwrap(),
        vec![name.to_owned(), "ab".to_owned()]
    );
    Objects::delete(&api, name).unwrap();
    assert_eq!(Objects::list(&api).unwrap(), vec!["ab".to_owned()]);

    let reqs = w.api_requests();
    let put = reqs.iter().find(|r| r.method == "PUT").unwrap();
    assert_eq!(put.path, format!("/api/v1/profiles/{name}"));
    assert_eq!(put.header("content-type"), Some("application/octet-stream"));
    assert!(reqs.iter().any(|r| r.path == "/api/v1/profiles"));
}

#[test]
fn an_unreachable_api_is_a_network_error() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::new(
        "http://127.0.0.1:9/realms/t",
        &format!("{}/api/v1", closed_base()),
    );
    let api = Api::new(&backend, &SessionStore::file(dir.path().into()), GOOD);
    let err = Remote::list(&api).unwrap_err();
    assert!(matches!(err, SyncError::Account(AccountError::Network(_))));
    assert_eq!(err.code(), "network");
}
