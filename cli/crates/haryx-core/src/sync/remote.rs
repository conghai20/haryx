//! The server side of sync: `public.sync_rows` through PostgREST, as the signed-in user.
//!
//! Row-level security limits every call to the caller's rows. Writes are optimistic: an update
//! names the version it read and raises it by one; if another machine got there first it updates
//! nothing, and the caller pulls again. [`Remote`] exists so the engine can be tested against an
//! in-memory server ([`Memory`]) as well as the real one.

use std::time::Duration;

use serde_json::{Value, json};

use super::SyncError;
use crate::account::AccountError;
use crate::oauth::http::{self, FetchError};

const TIMEOUT: Duration = Duration::from_secs(20);
const TABLE: &str = "sync_rows";
const KINDS: &str = "(keyring,settings,app-prefs,account,credential,profiles)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: String,
    pub key: String,
    pub body: String,
    pub version: i64,
    /// Unix seconds, set by the server.
    pub updated_at: i64,
}

pub trait Remote {
    fn list(&self) -> Result<Vec<Row>, SyncError>;
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

/// The Supabase project, as the user whose access token this is.
pub struct Postgrest {
    base: String,
    key: String,
    bearer: String,
}

impl Postgrest {
    pub fn new(base: &str, publishable_key: &str, access_token: &str) -> Self {
        Postgrest {
            base: format!("{}/rest/v1/{TABLE}", base.trim_end_matches('/')),
            key: publishable_key.to_owned(),
            bearer: format!("Bearer {access_token}"),
        }
    }

    fn headers<'a>(&'a self, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        let mut headers = vec![
            ("apikey", self.key.as_str()),
            ("authorization", self.bearer.as_str()),
            ("content-type", "application/json"),
            ("accept", "application/json"),
        ];
        headers.extend_from_slice(extra);
        headers
    }
}

fn filter(kind: &str, key: &str) -> String {
    format!("kind=eq.{}&key=eq.{}", encode(kind), encode(key))
}

/// Kinds and keys are `[a-z-]` and lowercase hex; anything else is encoded rather than trusted.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'0'..=b'9' | b'-' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn fetch_error(err: FetchError) -> SyncError {
    match &err {
        FetchError::Http { status: 401, .. } => SyncError::Account(AccountError::SignedOut(
            "the server no longer accepts this session".into(),
        )),
        FetchError::Http { status, body, .. } => SyncError::Server(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&body[..body.len().min(300)])
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
        )),
        _ => SyncError::Account(AccountError::Network(err.to_string())),
    }
}

fn rows(value: &Value) -> Result<Vec<Row>, SyncError> {
    let bad = || SyncError::Server("unexpected sync_rows response".into());
    value
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|row| {
            let text = |k: &str| row.get(k).and_then(Value::as_str).map(str::to_owned);
            let updated = text("updated_at").ok_or_else(bad)?;
            Ok(Row {
                kind: text("kind").ok_or_else(bad)?,
                key: text("key").ok_or_else(bad)?,
                body: text("body").ok_or_else(bad)?,
                version: row.get("version").and_then(Value::as_i64).ok_or_else(bad)?,
                updated_at: parse_time(&updated).ok_or_else(bad)?,
            })
        })
        .collect()
}

/// PostgREST's `timestamptz` (`2026-11-25T10:00:00.123+00:00`) as Unix seconds.
fn parse_time(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(jiff::Timestamp::as_second)
}

impl Remote for Postgrest {
    fn list(&self) -> Result<Vec<Row>, SyncError> {
        let url = format!(
            "{}?select=kind,key,body,version,updated_at&order=kind,key",
            self.base
        );
        let resp = http::get(&url, &self.headers(&[]), TIMEOUT).map_err(fetch_error)?;
        rows(&resp.json().map_err(fetch_error)?)
    }

    fn insert(&self, kind: &str, key: &str, body: &str) -> Result<Option<Row>, SyncError> {
        let payload = json!({"kind": kind, "key": key, "body": body}).to_string();
        let headers = self.headers(&[("prefer", "return=representation")]);
        match http::post(&self.base, &headers, payload.as_bytes(), TIMEOUT) {
            Ok(resp) => Ok(rows(&resp.json().map_err(fetch_error)?)?.into_iter().next()),
            Err(FetchError::Http { status: 409, .. }) => Ok(None),
            Err(err) => Err(fetch_error(err)),
        }
    }

    fn update(
        &self,
        kind: &str,
        key: &str,
        body: &str,
        version: i64,
    ) -> Result<Option<Row>, SyncError> {
        let url = format!("{}?{}&version=eq.{version}", self.base, filter(kind, key));
        let payload = json!({"body": body, "version": version + 1}).to_string();
        let headers = self.headers(&[("prefer", "return=representation")]);
        let resp = http::patch(&url, &headers, payload.as_bytes(), TIMEOUT).map_err(fetch_error)?;
        Ok(rows(&resp.json().map_err(fetch_error)?)?.into_iter().next())
    }

    fn delete(&self, kind: &str, key: &str) -> Result<(), SyncError> {
        let url = format!("{}?{}", self.base, filter(kind, key));
        http::delete(&url, &self.headers(&[]), TIMEOUT)
            .map(drop)
            .map_err(fetch_error)
    }

    fn delete_all(&self) -> Result<(), SyncError> {
        // A filter is required (deletes without one are refused); this one matches every row.
        let url = format!("{}?kind=in.{KINDS}", self.base);
        http::delete(&url, &self.headers(&[]), TIMEOUT)
            .map(drop)
            .map_err(fetch_error)
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
    fn postgrest_times_parse_to_unix_seconds() {
        assert_eq!(parse_time("1970-01-01T00:00:10+00:00"), Some(10));
        assert_eq!(
            parse_time("2026-11-25T10:00:00.123456+00:00"),
            Some(1_795_600_800)
        );
        assert_eq!(parse_time("garbage"), None);
    }

    #[test]
    fn filters_encode_anything_unexpected() {
        assert_eq!(filter("app-prefs", "ab12"), "kind=eq.app-prefs&key=eq.ab12");
        assert_eq!(encode("a&b=c"), "a%26b%3Dc");
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
