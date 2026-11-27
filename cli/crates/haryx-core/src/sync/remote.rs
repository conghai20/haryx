//! The server side of sync: the HaryX API's `/sync/rows`, as the signed-in user.
//!
//! The API scopes every call to the caller's rows. Writes are optimistic: an update names the
//! version it read (`If-Match`) and the server raises it by one; if another machine got there first
//! the update is refused, and the caller pulls again. [`Remote`] exists so the engine can be tested
//! against an in-memory server ([`Memory`]) as well as the real one ([`Api`]).

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use super::SyncError;
use crate::account::{AccountError, Backend, SessionStore};
use crate::oauth::http::{self, FetchError, HttpResponse};

const TIMEOUT: Duration = Duration::from_secs(20);
/// The API's caps (`server/api`: the `sync_rows_limit` and `profile_files_limit` triggers).
const MAX_ROWS: usize = 500;
const MAX_PROFILE_FILES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: String,
    pub key: String,
    pub body: String,
    pub version: i64,
    /// Unix seconds, set by the server.
    pub updated_at: i64,
}

/// The rows of one kind, with the server's clock when it answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub rows: Vec<Row>,
    /// Unix seconds from the response's `Date` header, when it had a readable one.
    pub server_now: Option<i64>,
}

pub trait Remote {
    fn list(&self) -> Result<Vec<Row>, SyncError>;
    /// Every row of one kind. The default filters [`Remote::list`] and knows no server clock.
    fn list_kind(&self, kind: &str) -> Result<Listing, SyncError> {
        let rows = self
            .list()?
            .into_iter()
            .filter(|r| r.kind == kind)
            .collect();
        Ok(Listing {
            rows,
            server_now: None,
        })
    }
    /// `None`: a row with that kind and key already exists (another machine wrote it first).
    fn insert(&self, kind: &str, key: &str, body: &str) -> Result<Option<Row>, SyncError>;
    /// `None`: the row is no longer at `version` (or is gone).
    fn update(
        &self,
        kind: &str,
        key: &str,
        body: &str,
        version: i64,
    ) -> Result<Option<Row>, SyncError>;
    fn delete(&self, kind: &str, key: &str) -> Result<(), SyncError>;
    /// Every row of this user.
    fn delete_all(&self) -> Result<(), SyncError>;
}

/// The HaryX API, as the signed-in user. Also the store of profile files
/// ([`super::profiles::Objects`]).
///
/// A 401 renews the session once ([`SessionStore::renew`]) and retries once. Only the sign-in
/// server decides that this machine is signed out (the Lessons rule): a refused renewal
/// (`invalid_grant`) forgets the session and is [`AccountError::SignedOut`]. A 401 from the API
/// for a token Keycloak has just issued is an error, and the session is kept: Keycloak still
/// vouches for it, so the disagreement (a key rotation the API has not seen, clock skew, a realm
/// change) is the server's to fix, not a reason to sign every machine out.
pub struct Api {
    base: String,
    backend: Backend,
    sessions: SessionStore,
    token: Mutex<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Api {
    /// `access_token` is the one [`SessionStore::current`] handed out for `backend`.
    pub fn new(backend: &Backend, sessions: &SessionStore, access_token: &str) -> Self {
        Api {
            base: backend.api_url().to_owned(),
            backend: backend.clone(),
            sessions: sessions.clone(),
            token: Mutex::new(access_token.to_owned()),
        }
    }

    fn token(&self) -> String {
        self.token.lock().expect("token lock").clone()
    }

    fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        timeout: Duration,
        token: &str,
    ) -> Result<HttpResponse, FetchError> {
        let url = format!("{}{path}", self.base);
        let bearer = format!("Bearer {token}");
        let mut all = vec![("authorization", bearer.as_str())];
        all.extend_from_slice(headers);
        match method {
            Method::Get => http::get(&url, &all, timeout),
            Method::Post => http::post(&url, &all, body, timeout),
            Method::Put => http::put(&url, &all, body, timeout),
            Method::Delete => http::delete(&url, &all, timeout),
        }
    }

    /// The access token after the API refused `rejected`.
    fn renew(&self, rejected: &str) -> Result<String, SyncError> {
        let now = jiff::Timestamp::now().as_second();
        let session = self
            .sessions
            .renew(&self.backend, rejected, now)?
            .ok_or(SyncError::NotSignedIn)?;
        let mut token = self.token.lock().expect("token lock");
        token.clone_from(&session.access_token);
        Ok(session.access_token)
    }

    /// `path` (under the API base, e.g. `/sync/rows`) as the signed-in user. `Ok(None)`: the API
    /// answered one of the `expected` statuses; any other failure is an error.
    pub(crate) fn call(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        timeout: Duration,
        expected: &[u16],
    ) -> Result<Option<HttpResponse>, SyncError> {
        let token = self.token();
        match self.send(method, path, headers, body, timeout, &token) {
            Err(FetchError::Http { status: 401, .. }) => {}
            other => return outcome(other, expected),
        }
        let token = self.renew(&token)?;
        match self.send(method, path, headers, body, timeout, &token) {
            Err(FetchError::Http { status: 401, .. }) => Err(SyncError::Server(format!(
                "HTTP 401 for a sign-in renewed a moment ago. Try again later; if it persists, run \
                 '{} login' again",
                crate::PROG
            ))),
            other => outcome(other, expected),
        }
    }

    fn json(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        payload: &Value,
        expected: &[u16],
    ) -> Result<Option<HttpResponse>, SyncError> {
        let mut all = vec![("content-type", "application/json")];
        all.extend_from_slice(headers);
        let body = payload.to_string();
        self.call(method, path, &all, body.as_bytes(), TIMEOUT, expected)
    }

    fn get(&self, path: &str) -> Result<HttpResponse, SyncError> {
        self.call(Method::Get, path, &[], &[], TIMEOUT, &[])
            .map(|resp| resp.expect("no expected status"))
    }
}

fn outcome(
    result: Result<HttpResponse, FetchError>,
    expected: &[u16],
) -> Result<Option<HttpResponse>, SyncError> {
    match result {
        Ok(resp) => Ok(Some(resp)),
        Err(FetchError::Http { status, .. }) if expected.contains(&status) => Ok(None),
        Err(err) => Err(api_error(err)),
    }
}

/// A failed API call. Errors are `{"error": "<code>"}`; the codes are the API's contract.
fn api_error(err: FetchError) -> SyncError {
    let FetchError::Http { status, body, .. } = &err else {
        return SyncError::Account(AccountError::Network(err.to_string()));
    };
    let code = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    match (*status, code.as_str()) {
        (403, "consent_required") => SyncError::ConsentRequired,
        (422, "row_limit") => SyncError::Invalid(format!(
            "The server keeps at most {MAX_ROWS} synced items per HaryX ID, and yours has them \
             all. Stop syncing accounts you no longer use with '{} sync forget EMAIL'.",
            crate::PROG
        )),
        (422, "profile_limit") => SyncError::Invalid(format!(
            "The server keeps at most {MAX_PROFILE_FILES} profile files per HaryX ID. Remove one \
             with '{} sync profile remove NAME' first.",
            crate::PROG
        )),
        (413, _) => SyncError::Invalid(format!(
            "Too large for the sync server ({})",
            readable(&code, body)
        )),
        _ => SyncError::Server(format!("HTTP {status}: {}", readable(&code, body))),
    }
}

/// The error code, or the start of the body without control characters.
fn readable(code: &str, body: &[u8]) -> String {
    if !code.is_empty() {
        return code.chars().filter(|c| !c.is_control()).take(100).collect();
    }
    String::from_utf8_lossy(&body[..body.len().min(300)])
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

/// A path segment. Kinds are `[a-z-]` and keys lowercase hex; anything else is encoded rather
/// than trusted.
pub(crate) fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'0'..=b'9' | b'-' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `/sync/rows/{kind}/{key}`; the empty key keeps its trailing slash.
fn row_path(kind: &str, key: &str) -> String {
    format!("/sync/rows/{}/{}", encode(kind), encode(key))
}

fn row(value: &Value) -> Result<Row, SyncError> {
    let bad = || SyncError::Server("unexpected sync row in the response".into());
    let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(Row {
        kind: text("kind").ok_or_else(bad)?,
        key: text("key").ok_or_else(bad)?,
        body: text("body").ok_or_else(bad)?,
        version: value
            .get("version")
            .and_then(Value::as_i64)
            .ok_or_else(bad)?,
        updated_at: text("updatedAt")
            .as_deref()
            .and_then(parse_time)
            .ok_or_else(bad)?,
    })
}

fn rows(resp: &HttpResponse) -> Result<Vec<Row>, SyncError> {
    resp.json()
        .map_err(api_error)?
        .as_array()
        .ok_or_else(|| SyncError::Server("unexpected sync rows response".into()))?
        .iter()
        .map(row)
        .collect()
}

fn one_row(resp: &HttpResponse) -> Result<Row, SyncError> {
    row(&resp.json().map_err(api_error)?)
}

/// An HTTP `Date` header (`Thu, 26 Nov 2026 16:00:00 GMT`) as Unix seconds.
fn parse_http_date(text: &str) -> Option<i64> {
    jiff::fmt::rfc2822::parse(text.trim())
        .ok()
        .map(|zoned| zoned.timestamp().as_second())
}

/// An RFC 3339 time (`2026-11-25T10:00:00.123Z`) as Unix seconds.
fn parse_time(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(jiff::Timestamp::as_second)
}

impl Remote for Api {
    fn list(&self) -> Result<Vec<Row>, SyncError> {
        rows(&self.get("/sync/rows")?)
    }

    fn list_kind(&self, kind: &str) -> Result<Listing, SyncError> {
        let resp = self.get(&format!("/sync/rows?kind={}", encode(kind)))?;
        Ok(Listing {
            rows: rows(&resp)?,
            server_now: resp.headers.get("date").and_then(parse_http_date),
        })
    }

    fn insert(&self, kind: &str, key: &str, body: &str) -> Result<Option<Row>, SyncError> {
        let payload = json!({"kind": kind, "key": key, "body": body});
        self.json(Method::Post, "/sync/rows", &[], &payload, &[409])?
            .map(|resp| one_row(&resp))
            .transpose()
    }

    fn update(
        &self,
        kind: &str,
        key: &str,
        body: &str,
        version: i64,
    ) -> Result<Option<Row>, SyncError> {
        let version = version.to_string();
        let headers = [("if-match", version.as_str())];
        let payload = json!({"body": body});
        // 412: another machine moved the row; 404: it is gone. Either way, pull again.
        self.json(
            Method::Put,
            &row_path(kind, key),
            &headers,
            &payload,
            &[412, 404],
        )?
        .map(|resp| one_row(&resp))
        .transpose()
    }

    fn delete(&self, kind: &str, key: &str) -> Result<(), SyncError> {
        self.call(Method::Delete, &row_path(kind, key), &[], &[], TIMEOUT, &[])
            .map(drop)
    }

    fn delete_all(&self) -> Result<(), SyncError> {
        self.call(Method::Delete, "/sync/rows", &[], &[], TIMEOUT, &[])
            .map(drop)
    }
}

impl<R: Remote + ?Sized> Remote for std::sync::Arc<R> {
    fn list(&self) -> Result<Vec<Row>, SyncError> {
        (**self).list()
    }
    fn list_kind(&self, kind: &str) -> Result<Listing, SyncError> {
        (**self).list_kind(kind)
    }
    fn insert(&self, kind: &str, key: &str, body: &str) -> Result<Option<Row>, SyncError> {
        (**self).insert(kind, key, body)
    }
    fn update(
        &self,
        kind: &str,
        key: &str,
        body: &str,
        version: i64,
    ) -> Result<Option<Row>, SyncError> {
        (**self).update(kind, key, body, version)
    }
    fn delete(&self, kind: &str, key: &str) -> Result<(), SyncError> {
        (**self).delete(kind, key)
    }
    fn delete_all(&self) -> Result<(), SyncError> {
        (**self).delete_all()
    }
}

/// An in-memory server with the same rules: versions start at 1 and step by one.
#[derive(Default)]
pub struct Memory {
    rows: std::sync::Mutex<Vec<Row>>,
    clock: std::sync::atomic::AtomicI64,
}

impl Memory {
    fn tick(&self) -> i64 {
        1_800_000_000 + self.clock.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    pub fn snapshot(&self) -> Vec<Row> {
        self.rows.lock().expect("rows lock").clone()
    }
}

impl Remote for Memory {
    fn list(&self) -> Result<Vec<Row>, SyncError> {
        Ok(self.snapshot())
    }

    fn list_kind(&self, kind: &str) -> Result<Listing, SyncError> {
        let rows = self
            .snapshot()
            .into_iter()
            .filter(|r| r.kind == kind)
            .collect();
        let now = 1_800_000_000 + self.clock.load(std::sync::atomic::Ordering::SeqCst);
        Ok(Listing {
            rows,
            server_now: Some(now),
        })
    }

    fn insert(&self, kind: &str, key: &str, body: &str) -> Result<Option<Row>, SyncError> {
        let mut rows = self.rows.lock().expect("rows lock");
        if rows.iter().any(|r| r.kind == kind && r.key == key) {
            return Ok(None);
        }
        let row = Row {
            kind: kind.into(),
            key: key.into(),
            body: body.into(),
            version: 1,
            updated_at: self.tick(),
        };
        rows.push(row.clone());
        Ok(Some(row))
    }

    fn update(
        &self,
        kind: &str,
        key: &str,
        body: &str,
        version: i64,
    ) -> Result<Option<Row>, SyncError> {
        let now = self.tick();
        let mut rows = self.rows.lock().expect("rows lock");
        let Some(row) = rows
            .iter_mut()
            .find(|r| r.kind == kind && r.key == key && r.version == version)
        else {
            return Ok(None);
        };
        row.body = body.into();
        row.version += 1;
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    fn delete(&self, kind: &str, key: &str) -> Result<(), SyncError> {
        self.rows
            .lock()
            .expect("rows lock")
            .retain(|r| !(r.kind == kind && r.key == key));
        Ok(())
    }

    fn delete_all(&self) -> Result<(), SyncError> {
        self.rows.lock().expect("rows lock").clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_times_parse_to_unix_seconds() {
        assert_eq!(parse_time("1970-01-01T00:00:10Z"), Some(10));
        assert_eq!(
            parse_time("2026-11-25T10:00:00.123456Z"),
            Some(1_795_600_800)
        );
        assert_eq!(parse_time("garbage"), None);
    }

    #[test]
    fn http_dates_parse_to_unix_seconds() {
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:10 GMT"), Some(10));
        assert_eq!(parse_http_date("yesterday"), None);
    }

    #[test]
    fn paths_encode_anything_unexpected() {
        assert_eq!(row_path("app-prefs", "ab12"), "/sync/rows/app-prefs/ab12");
        assert_eq!(row_path("settings", ""), "/sync/rows/settings/");
        assert_eq!(encode("a/b?c"), "a%2Fb%3Fc");
    }

    #[test]
    fn memory_enforces_one_step_versions() {
        let m = Memory::default();
        let row = m.insert("settings", "", "a").unwrap().unwrap();
        assert_eq!(row.version, 1);
        assert!(m.insert("settings", "", "b").unwrap().is_none());
        assert_eq!(
            m.update("settings", "", "b", 1).unwrap().unwrap().version,
            2
        );
        assert!(m.update("settings", "", "c", 1).unwrap().is_none());
        m.delete("settings", "").unwrap();
        assert!(m.list().unwrap().is_empty());
    }
}
