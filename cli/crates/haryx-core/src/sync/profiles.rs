//! Setup profiles carried between machines: `haryx sync profile push|pull|list`.
//!
//! A profile travels as its export file (secrets redacted, no history, no credentials), encrypted
//! whole to the sync key, in the private `profiles` bucket at `<user id>/<HMAC of the name>`. Names
//! live only in the encrypted `profiles` index row. A pull hands back the file for
//! `ai profile import`; applying it stays the explicit `ai profile apply`.

use std::time::Duration;

use serde_json::json;

use super::crypto::SyncKey;
use super::merge::Values;
use super::remote::Row;
use super::{Context, KIND_PROFILES, Opened, SyncError, open, require_key, write_row};
use crate::account::AccountError;
use crate::oauth::http::{self, FetchError};

/// Profiles one account may keep on the server.
pub const MAX_PROFILES: usize = 3;
/// Largest encrypted profile file; the bucket enforces the same limit.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
const BUCKET: &str = "profiles";

/// Where the encrypted files live.
pub trait Objects {
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), SyncError>;
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError>;
    fn delete(&self, name: &str) -> Result<(), SyncError>;
}

/// Supabase Storage, as the signed-in user. Object names start with the user id; the bucket's
/// policies refuse any other prefix.
pub struct Storage {
    base: String,
    key: String,
    bearer: String,
}

impl Storage {
    pub fn new(project: &str, publishable_key: &str, access_token: &str) -> Self {
        Storage {
            base: format!("{}/storage/v1/object", project.trim_end_matches('/')),
            key: publishable_key.to_owned(),
            bearer: format!("Bearer {access_token}"),
        }
    }

    fn headers<'a>(&'a self, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        let mut headers = vec![
            ("apikey", self.key.as_str()),
            ("authorization", self.bearer.as_str()),
        ];
        headers.extend_from_slice(extra);
        headers
    }
}

fn storage_error(err: FetchError) -> SyncError {
    match &err {
        FetchError::Http { status: 401, .. } => SyncError::Account(AccountError::SignedOut(
            "the server no longer accepts this session".into(),
        )),
        FetchError::Http { status, body, .. } => SyncError::Server(format!(
            "storage: HTTP {status}: {}",
            String::from_utf8_lossy(&body[..body.len().min(300)])
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
        )),
        _ => SyncError::Account(AccountError::Network(err.to_string())),
    }
}

impl Objects for Storage {
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), SyncError> {
        let url = format!("{}/{BUCKET}/{name}", self.base);
        let headers = self.headers(&[
            ("content-type", "application/octet-stream"),
            ("x-upsert", "true"),
        ]);
        http::post(&url, &headers, bytes, TIMEOUT)
            .map(drop)
            .map_err(storage_error)
    }

    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
        let url = format!("{}/authenticated/{BUCKET}/{name}", self.base);
        match http::get(&url, &self.headers(&[]), TIMEOUT) {
            Ok(resp) => Ok(Some(resp.body)),
            Err(FetchError::Http {
                status: 400 | 404, ..
            }) => Ok(None),
            Err(err) => Err(storage_error(err)),
        }
    }

    fn delete(&self, name: &str) -> Result<(), SyncError> {
        let url = format!("{}/{BUCKET}/{name}", self.base);
        match http::delete(&url, &self.headers(&[]), TIMEOUT) {
            Ok(_)
            | Err(FetchError::Http {
                status: 400 | 404, ..
            }) => Ok(()),
            Err(err) => Err(storage_error(err)),
        }
    }
}

/// Objects in memory, for tests.
#[derive(Default)]
pub struct MemoryObjects {
    pub files: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl Objects for MemoryObjects {
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), SyncError> {
        self.files
            .lock()
            .expect("files lock")
            .insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
        Ok(self.files.lock().expect("files lock").get(name).cloned())
    }

    fn delete(&self, name: &str) -> Result<(), SyncError> {
        self.files.lock().expect("files lock").remove(name);
        Ok(())
    }
}

/// A profile on the server.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub bytes: u64,
    /// Unix seconds.
    pub pushed_at: i64,
}

fn object_name(ctx: &Context<'_>, key: &SyncKey, name: &str) -> String {
    format!("{}/{}", ctx.user, key.row_key(&format!("profile:{name}")))
}

/// The index row and its entries, by object key. An index that does not open reads as empty, and
/// the next push replaces it.
fn index(ctx: &Context<'_>, key: &SyncKey) -> Result<(Option<Row>, Values), SyncError> {
    let row = ctx
        .remote
        .list()?
        .into_iter()
        .find(|r| r.kind == KIND_PROFILES && r.key.is_empty());
    let values = match row.as_ref().map(|row| open(key, row)) {
        Some(Opened::Doc(doc)) => doc.values,
        _ => Values::new(),
    };
    Ok((row, values))
}

/// What is encrypted for profile `name`: the file, behind a header naming it, so the server cannot
/// hand back one profile's file as another's.
fn framed(name: &str, file: &[u8]) -> Vec<u8> {
    let mut plain = format!("haryx-profile:{name}\n").into_bytes();
    plain.extend_from_slice(file);
    plain
}

fn entries(values: &Values) -> Vec<Entry> {
    let mut list: Vec<Entry> = values
        .values()
        .filter_map(|v| {
            Some(Entry {
                name: v.get("name")?.as_str()?.to_owned(),
                bytes: v.get("bytes")?.as_u64()?,
                pushed_at: v.get("pushedAt")?.as_i64()?,
            })
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// Write the index, at the version it was read. `false`: another machine changed it first.
fn save_index(
    ctx: &Context<'_>,
    key: &SyncKey,
    row: Option<&Row>,
    values: Values,
) -> Result<bool, SyncError> {
    let current = row.map(|r| r.version);
    Ok(write_row(ctx, key, KIND_PROFILES, "", values, ctx.now, current)?.is_some())
}

pub fn list(ctx: &Context<'_>) -> Result<Vec<Entry>, SyncError> {
    let key = require_key(ctx)?;
    Ok(entries(&index(ctx, &key)?.1))
}

/// Upload the export file of profile `name`, replacing an earlier one of that name.
pub fn push(ctx: &Context<'_>, name: &str, file: &[u8]) -> Result<Entry, SyncError> {
    let key = require_key(ctx)?;
    let cipher = key.seal_bytes(&framed(name, file))?;
    if cipher.len() > MAX_BYTES {
        return Err(SyncError::Invalid(format!(
            "Profile '{name}' is {} MB encrypted; the limit is {} MB. Leave plugins as references \
             and history out.",
            cipher.len() / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        )));
    }
    let object = object_name(ctx, &key, name);
    let object_key = object.rsplit('/').next().unwrap_or_default().to_owned();
    for _ in 0..3 {
        let (row, mut values) = index(ctx, &key)?;
        if !values.contains_key(&object_key) && values.len() >= MAX_PROFILES {
            return Err(SyncError::Invalid(format!(
                "At most {MAX_PROFILES} profiles are kept on the server; remove one with \
                 'haryx sync profile remove NAME' first"
            )));
        }
        ctx.objects.put(&object, &cipher)?;
        let entry = Entry {
            name: name.to_owned(),
            bytes: file.len() as u64,
            pushed_at: ctx.now,
        };
        values.insert(
            object_key.clone(),
            json!({"name": entry.name, "bytes": entry.bytes, "pushedAt": entry.pushed_at}),
        );
        if save_index(ctx, &key, row.as_ref(), values)? {
            return Ok(entry);
        }
    }
    Err(SyncError::Busy)
}

/// The export file of profile `name`, decrypted. `None`: no such profile on the server.
pub fn pull(ctx: &Context<'_>, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
    let key = require_key(ctx)?;
    let object = object_name(ctx, &key, name);
    let Some(cipher) = ctx.objects.get(&object)? else {
        return Ok(None);
    };
    let header = framed(name, b"");
    match key.open_bytes(&cipher) {
        Some(plain) if plain.starts_with(&header) => Ok(Some(plain[header.len()..].to_vec())),
        _ => Err(SyncError::Server(format!(
            "the server's copy of profile '{name}' does not open with this sync key; push it again"
        ))),
    }
}

/// Remove profile `name` from the server. `false`: there was none.
pub fn remove(ctx: &Context<'_>, name: &str) -> Result<bool, SyncError> {
    let key = require_key(ctx)?;
    let object = object_name(ctx, &key, name);
    let object_key = object.rsplit('/').next().unwrap_or_default().to_owned();
    for _ in 0..3 {
        let (row, mut values) = index(ctx, &key)?;
        if values.remove(&object_key).is_none() {
            return Ok(false);
        }
        ctx.objects.delete(&object)?;
        if save_index(ctx, &key, row.as_ref(), values)? {
            return Ok(true);
        }
    }
    Err(SyncError::Busy)
}

/// Delete every profile file this account has (`sync reset`). Needs the key to read the index; a
/// machine without it can only clear the rows, and the bucket's files are then orphaned until the
/// account is deleted.
pub fn delete_all_objects(ctx: &Context<'_>) -> Result<(), SyncError> {
    let Ok(key) = require_key(ctx) else {
        return Ok(());
    };
    let Ok((_, values)) = index(ctx, &key) else {
        return Ok(());
    };
    for object_key in values.keys() {
        ctx.objects.delete(&format!("{}/{object_key}", ctx.user))?;
    }
    Ok(())
}
